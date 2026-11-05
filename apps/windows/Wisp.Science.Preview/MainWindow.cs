using System.Diagnostics;
using System.Text.Json.Nodes;
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
    private readonly Grid windowFrame = new();
    // A modal's viewport includes the custom title bar, just like a fixed
    // WebView overlay. Full-page sheets stay inside the workbench row.
    private readonly Grid modalLayer = new();
    private Grid SheetHost(IWorkspaceSheet sheet) => sheet.IsModal ? modalLayer : root;
    private readonly NativeWindowTitlebar windowTitlebar;
    private string? hostVersion;
    private readonly WispDesign design = new();
    private readonly PreviewSettings settings = PreviewSettings.Load();
    private readonly ProjectBrowserModel model;
    private readonly ProjectBrowserClient browserClient;
    private Task conversationReady = Task.CompletedTask;
    private readonly NativeProjectInventory projectInventory = new();
    private TextBlock? inventoryLabel;
    private readonly List<FlyoutBase> overlays = [];
    private ProjectSearchOverlay? searchOverlay;
    private Control? searchPreviousFocus;
    private FrameworkElement? pageContent;
    private NativeSettingsPage? settingsPage;
    private string? settingsAssistantProject, settingsAssistantSession;
    private IWorkspaceSheet? workspaceSheet;
    private NativeSettingsClient? workspaceHost;
    private Task<NativeSettingsClient?>? connectingHost;
    private int conversationNavigation;
    private int runNavigation;
    private WorkspaceInboxModel? inbox;
    private NativeWorkspacePanel? panelPage;
    private NativeWorkspaceTerminal? terminalPage;
    private WorkspaceConversationModel? conversation;
    private NativeConversationPage? conversationPage;
    private WorkspaceSideChatModel? sideChat;
    private WorkspaceMemosModel? projectMemos;
    private bool panelVisible;
    private bool terminalVisible;
    private string? workspaceKey;
    private bool windowClosed;
    private readonly NativeWindowEscape nativeEscape;
    private bool sidebarVisible = true;
    private Action? refreshSidebarNavigation;
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
    private Grid? workspaceToolbar;
    private Grid? workspaceShell;
    private Grid? workspacePanelHost;
    private bool workspacePanelOverlay;
    private bool changingAppearance;
    private int appearanceGeneration;
    private Task<bool> appearancePersistence = Task.FromResult(true);
    private NativeSettingsClient? appearanceModelHost;
    private readonly Dictionary<string, NativeLiveAppearanceModel> appearanceModels = [];
    private NativeStorageBrowser storageBrowser = new();
    private NativeSessionTreeView? sessionTree;

    [System.Runtime.InteropServices.DllImport("user32.dll")]
    private static extern uint GetDpiForWindow(nint hwnd);

    private void SizeWindowForDisplay()
    {
        var screen = Microsoft.UI.Windowing.DisplayArea.GetFromWindowId(AppWindow.Id,
            Microsoft.UI.Windowing.DisplayAreaFallback.Nearest);
        var work = screen.WorkArea;
        var outer = AppWindow.Size; var client = AppWindow.ClientSize;
        var dpi = GetDpiForWindow(WinRT.Interop.WindowNative.GetWindowHandle(this));
        var (width, height) = PreviewLayout.InitialWindowSize(dpi, work.Width, work.Height,
            outer.Width - client.Width, outer.Height - client.Height);
        var left = screen.OuterBounds.X + work.X; var top = screen.OuterBounds.Y + work.Y;
        var x = Math.Clamp(AppWindow.Position.X, left, left + work.Width - width);
        var y = Math.Clamp(AppWindow.Position.Y, top, top + work.Height - height);
        AppWindow.MoveAndResize(new Windows.Graphics.RectInt32(x, y, width, height));
    }

    public MainWindow(string? database = null, string? initialProject = null, string? initialSession = null)
    {
        Title = NativeHomePresentation.WindowTitle(null);
        windowFrame.RowDefinitions.Add(new() { Height = GridLength.Auto });
        windowFrame.RowDefinitions.Add(new() { Height = new GridLength(1, GridUnitType.Star) });
        Grid.SetRow(root, 1); windowFrame.Children.Add(root); Content = windowFrame;
        design.Typography = settings.Typography ?? new();
        design.SelectionPopupEnabled = (settings.InputPreferences ?? new()).SelectionPopupEnabled;
        design.ContextUsageLayout.Geometry = settings.ContextUsageGeometry is { Valid: true } geometry ? geometry : null;
        design.SaveContextUsageGeometry = geometry => { settings.ContextUsageGeometry = geometry; SaveSettings(); };
        design.ComposerTextHeight = settings.ComposerTextHeight is { } savedHeight ? PreviewLayout.ResizeComposer(savedHeight, 0) : null;
        design.SaveComposerTextHeight = value => { settings.ComposerTextHeight = value; SaveSettings(); };
        root.KeyboardAcceleratorPlacementMode = KeyboardAcceleratorPlacementMode.Hidden;
        windowFrame.RequestedTheme = settings.Appearance switch { "light" => ElementTheme.Light, "dark" => ElementTheme.Dark, _ => ElementTheme.Default };
        panelVisible = settings.PanelVisible;
        browserClient = new(Environment.GetEnvironmentVariable("WISP_SERVICE_PATH") ?? Path.Combine(AppContext.BaseDirectory, "wisp-service.exe"));
        model = new(browserClient, database ?? settings.ResolveDatabase());
        windowTitlebar = new(this, design, RunWindowCommand, CanRunWindowCommand, Register);
        SizeWindowForDisplay();
        windowFrame.Children.Add(windowTitlebar);
        Grid.SetRowSpan(modalLayer, 2); windowFrame.Children.Add(modalLayer);
        windowFrame.KeyboardAcceleratorPlacementMode = KeyboardAcceleratorPlacementMode.Hidden;
        model.Changed += Render;
        homePoll.Tick += (_, _) => _ = ReadHomeActivityAsync();
        model.HomeActivityChanged += UpdateHomeActivityRows;
        Activated += (_, e) =>
        {
            homeForeground = e.WindowActivationState != WindowActivationState.Deactivated;
            if (homeForeground) _ = RefreshHomePrivacyAsync();
            UpdateHomePolling();
        };
        root.ActualThemeChanged += (_, _) => Render();
        root.SizeChanged += (_, e) =>
        {
            if (terminalPage != null) terminalPage.MaxHeight = PreviewLayout.TerminalMaxHeight(e.NewSize.Height);
            var next = PreviewLayout.ForSize(e.NewSize.Width, e.NewSize.Height);
            if (next != layout) { layout = next; Render(); }
        };
        Shortcut(VirtualKey.K, VirtualKeyModifiers.Control, OpenSearch, overChatHistory: true);
        Shortcut(VirtualKey.N, VirtualKeyModifiers.Control, () => RunWindowCommand(model.ActiveProjectId == null ? "new-project" : "new"));
        Shortcut(VirtualKey.P, VirtualKeyModifiers.Control, () => OpenSearch(true), overChatHistory: true, commands: true);
        Shortcut(VirtualKey.P, VirtualKeyModifiers.Control | VirtualKeyModifiers.Shift, () => OpenSearch(true), overChatHistory: true, commands: true);
        var historyShortcut = new KeyboardAccelerator { Key = VirtualKey.F, Modifiers = VirtualKeyModifiers.Control | VirtualKeyModifiers.Shift };
        historyShortcut.Invoked += (_, e) => e.Handled = OpenChatHistory();
        windowFrame.KeyboardAccelerators.Add(historyShortcut);
        Shortcut(VirtualKey.H, VirtualKeyModifiers.Control | VirtualKeyModifiers.Shift, () => _ = OpenPrivacyAsync(), commands: true);
        Shortcut(VirtualKey.B, VirtualKeyModifiers.Control, () => { if (model.ActiveProjectId != null) RunPaletteCommand("toggle-sidebar"); });
        Shortcut((VirtualKey)188, VirtualKeyModifiers.Control, () => RunWindowCommand("settings"));
        Shortcut(VirtualKey.R, VirtualKeyModifiers.Control, () => _ = model.RefreshAsync());
        Shortcut(VirtualKey.O, VirtualKeyModifiers.Control, ChooseDatabase);
        // Window-root accelerator also works immediately after opening a surface.
        // Native text menus handle Escape first; registered app flyouts precede the dialog.
        var escape = new KeyboardAccelerator { Key = VirtualKey.Escape };
        escape.Invoked += (_, e) => e.Handled = HandleWindowEscape();
        windowFrame.KeyboardAccelerators.Add(escape);
        var menus = new KeyboardAccelerator { Key = VirtualKey.F10 };
        menus.Invoked += (_, e) => { windowTitlebar.Open("file"); e.Handled = true; };
        windowFrame.KeyboardAccelerators.Add(menus);
        nativeEscape = new(WinRT.Interop.WindowNative.GetWindowHandle(this), HandleWindowEscape);
        Closed += (_, _) =>
        {
            nativeEscape.Dispose();
            homePoll.Stop(); homeRead?.Cancel(); model.HomeActivityChanged -= UpdateHomeActivityRows;
            chatSearchNotice.Hide();
            windowTitlebar.Dispose();
            windowClosed = true; model.Changed -= Render; model.Dispose();
            examplePage?.Dispose();
            foreach (var conversion in workflowConversions.Values) conversion.Dispose();
            DisposeWorkspace(); conversationPage?.Dispose(); conversation?.Pause();
            settingsPage?.Dispose(); searchOverlay?.Dispose(); projectImport?.Dispose(); projectExport?.Dispose(); workspaceHost?.Dispose();
        };
        root.Loaded += async (_, _) =>
        {
            var revision = model.NavigationRevision;
            await model.RefreshAsync();
            if (windowClosed) return;
            var hostConnection = ConnectHostAsync();
            // First-run links must wait for the host to create the database and
            // recover the initial read. Existing databases remain navigable now.
            if (model.LastLoaded == null) await hostConnection;
            if (!windowClosed && revision == model.NavigationRevision && initialProject != null)
                await model.OpenProjectAsync(initialProject, initialSession);
            await hostConnection;
            if (!windowClosed && revision == model.NavigationRevision && initialProject == null && quickSetupIntent == 0)
                await OpenQuickSetupAsync(firstLaunch: true);
        };
        Render();
    }

    // WebView2 owns keyboard focus inside local PDF previews. Forward its Escape
    // into the same stack as the window accelerator, preserving topmost order.
    private bool HandleWindowEscape()
    {
        if (nativePickerOpen || windowClosed) return false;
        if (conversationPage?.HandleFullPermissionEscape() == true) return true;
        if (overlays.LastOrDefault() is { } flyout) { flyout.Hide(); return true; }
        if (conversationPage?.HandleContextRecoveryEscape() == true) return true;
        if (conversationPage?.HandleRuntimeEscape() == true) return true;
        if (NativeWindowEscape.DispatchPages(settingsPage is { } settings ? settings.HandleEscape : null,
            workspaceSheet is { } sheet ? sheet.HandleEscape : null, searchOverlay != null ? CloseSearch : null,
            workspaceSheet is NativeQuickSetupPage or NativePrivacyPage ? workspaceSheet.HandleEscape : null)) return true;
        if (examplePage != null) return examplePage.HandleEscape();
        if (projectImport?.HandleEscape() == true) return true;
        if (projectExport?.HandleEscape() == true) return true;
        if (projectPage != null) { projectPage.HandleEscape(); return true; }
        if (panelPage?.HandleArtifactEscape() == true) return true;
        if (centerFiles?.HandleEscape() == true) return true;
        if (conversationPage?.HandleComposerTextEscape() == true) return true;
        if (conversationPage?.HandleAttachmentEscape() == true) return true;
        if (conversationPage?.HandleQueueEscape() == true) return true;
        if (conversationPage?.HandleHistoryEscape() == true) return true;
        if (conversationPage?.HandleReadingEscape() == true) return true;
        if (conversationPage?.HandleAcpEscape() == true) return true;
        if (conversationPage?.HandleComposeMenuEscape() == true) return true;
        if (conversationPage?.HandleComposerOptionsEscape() == true) return true;
        if (panelVisible && panelPage?.HandleEscape() == true) return true;
        if (panelVisible && workspacePanelOverlay) { TogglePanel(); return true; }
        if (terminalVisible && terminalPage?.HandleEscape() == true) return true;
        return conversationPage?.HandleEscape() == true;
    }

    private bool SetupCommandLayer => workspaceSheet is NativeQuickSetupPage { CanLeave: true }
        || settingsPage != null && workspaceSheet is (null or NativeAssistantPage);
    private void Shortcut(VirtualKey key, VirtualKeyModifiers modifiers, Action action, bool overChatHistory = false, bool commands = false)
    {
        var accelerator = new KeyboardAccelerator { Key = key, Modifiers = modifiers };
        accelerator.Invoked += (_, e) =>
        {
            var overSettings = commands && SetupCommandLayer && !overlays.Any();
            if (examplePage != null || nativePickerOpen || conversationPage?.HasRuntimeModal == true || searchOverlay != null || settingsPage != null && !overSettings
                || workspaceSheet != null && !overSettings && !(overChatHistory && workspaceSheet is NativeProjectChatSearch)
                || projectImport?.IsModal == true || projectExport?.IsModal == true) return;
            action(); e.Handled = true;
        };
        windowFrame.KeyboardAccelerators.Add(accelerator);
    }

    private void Render()
    {
        if (model.ActiveProjectId != null || examplePage != null) homeSyncNotice = null;
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
            modalLayer.Children.Clear();
            root.Children.Clear();
            var recovery = Stack(12); recovery.Margin = new Thickness(24);
            recovery.Children.Add(Text(localError, 14, "clay-strong"));
            recovery.Children.Add(ActionButton(design.T("sidebar.back_projects"), "arrow-left", model.GoHome, true));
            root.Children.Add(recovery);
        }
        finally { RefreshWindowTitlebar(); RefreshWorkflowConversionNotice(); design.ApplyTypography(root, settingsPage); rendering = false; UpdateHomePolling(); }
    }

    private void RenderCore()
    {
        if (newProjectDraft is { } saved && (saved.Database != model.DatabasePath || model.ActiveProjectId != null || examplePage != null))
            newProjectDraft = null;
        if (settingsPage != null) return;
        if (workspaceSheet is NativeJourneyPage { KeepsProjectSidebar: true } journey
            && (journey.ProjectId != model.ActiveProjectId || renderedSession != model.ActiveSessionId)) ClearSheets();
        sessionTree = null;
        SyncWorkspaceSession();
        var offset = transcriptScroll?.VerticalOffset ?? 0;
        var height = transcriptScroll?.ExtentHeight ?? 0;
        var oldSession = renderedSession;
        var oldFirst = renderedFirst;
        design.LightPalette = settings.LightPalette;
        design.DarkPalette = settings.DarkPalette;
        design.Dark = root.ActualTheme == ElementTheme.Dark;
        design.RefreshPalette();
        root.Background = design.Brush("bg-app");
        refreshSidebarNavigation = null;
        updateHomeRows = null;
        // WinUI can report Parent=null while an unloaded subtree still owns a
        // reusable UserControl. Detach through the retained owner before remounting.
        workspaceToolbar?.Children.Clear(); workspaceToolbar = null;
        workspaceMain?.Children.Clear(); workspaceMain = null;
        workspacePanelHost?.Children.Clear(); workspacePanelHost = null;
        workspacePanelOverlay = false;
        workspaceShell?.Children.Clear(); workspaceShell = null;
        modalLayer.Children.Clear();
        root.Children.Clear();
        transcriptScroll = null;
        var project = model.Projects.FirstOrDefault(p => p.Id == model.ActiveProjectId);
        Title = NativeHomePresentation.WindowTitle(examplePage != null ? design.T("projects.example") : project?.Name);
        pageContent = examplePage ?? (project == null ? Home() : Workspace(project));
        pageContent.IsHitTestVisible = searchOverlay == null && workspaceSheet == null && projectImport?.IsModal != true && projectExport?.IsModal != true;
        pageContent.Visibility = workspaceSheet == null || workspaceSheet.IsModal ? Visibility.Visible : Visibility.Collapsed;
        root.Children.Add(pageContent);
        if (projectImport != null) root.Children.Add(projectImport);
        if (projectExport != null) root.Children.Add(projectExport);
        foreach (var parent in parentSheets.Reverse())
            if (parent.Retained && parent.Sheet is UserControl retained) SheetHost(parent.Sheet).Children.Add(retained);
        if (workspaceSheet is UserControl sheet) SheetHost(workspaceSheet).Children.Add(sheet);
        if (searchOverlay != null) modalLayer.Children.Add(searchOverlay);
        UpdateSheetPresentation();
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
                if (savedChatRow is { } match && scrollToChatHit)
                {
                    scrollToChatHit = false; scrollToLatest = false;
                    match.StartBringIntoView(new BringIntoViewOptions { VerticalAlignmentRatio = 0 });
                }
                else if (scrollToLatest) { scroll.ChangeView(null, scroll.ScrollableHeight, null, true); scrollToLatest = false; }
                else scroll.ChangeView(null, offset + (oldFirst != renderedFirst ? Math.Max(0, scroll.ExtentHeight - height) : 0), null, true);
            };
            scroll.LayoutUpdated += restore;
            scroll.Unloaded += (_, _) => scroll.LayoutUpdated -= restore;
        }
    }

    private FrameworkElement Home()
    {
        // One page scroller keeps both lists and the footer reachable in short windows.
        // The brand never shrinks just because Windows scaling reduces client height.
        var scroll = new ScrollViewer { HorizontalScrollBarVisibility = ScrollBarVisibility.Disabled, HorizontalScrollMode = ScrollMode.Disabled,
            HorizontalContentAlignment = HorizontalAlignment.Stretch, VerticalScrollBarVisibility = ScrollBarVisibility.Auto };
        AutomationProperties.SetAutomationId(scroll, "projects-home");
        var page = new Grid { HorizontalAlignment = HorizontalAlignment.Left };
        page.RowDefinitions.Add(new() { Height = GridLength.Auto });
        page.RowDefinitions.Add(new() { Height = GridLength.Auto });
        page.RowDefinitions.Add(new() { Height = new GridLength(1, GridUnitType.Star) });
        page.RowDefinitions.Add(new() { Height = GridLength.Auto });
        var header = new Grid { ColumnSpacing = HomeLayout.HeaderGap, RowSpacing = 16, Margin = new Thickness(0, 0, 0, HomeLayout.SectionGap) };
        header.ColumnDefinitions.Add(new()); header.ColumnDefinitions.Add(new());
        header.RowDefinitions.Add(new() { Height = GridLength.Auto });
        header.RowDefinitions.Add(new() { Height = GridLength.Auto });
        var brand = new Grid { ColumnSpacing = HomeLayout.HeaderGap, RowSpacing = 12 };
        brand.ColumnDefinitions.Add(new()); brand.ColumnDefinitions.Add(new());
        brand.RowDefinitions.Add(new() { Height = GridLength.Auto });
        brand.RowDefinitions.Add(new() { Height = GridLength.Auto });
        var mark = design.Wordmark();
        mark.Width = HomeLayout.WordmarkWidth; mark.Height = HomeLayout.WordmarkHeight;
        mark.HorizontalAlignment = HorizontalAlignment.Left;
        AutomationProperties.SetName(mark, "Wisp Science");
        brand.Children.Add(mark);
        var tagline = Stack(0); tagline.VerticalAlignment = VerticalAlignment.Center;
        tagline.Children.Add(Text(design.T("projects.tagline_lead"), 16, "text-muted"));
        tagline.Children.Add(Text(design.T("projects.tagline_brand"), 16, "clay-strong"));
        foreach (var line in tagline.Children.OfType<TextBlock>())
        {
            line.FontWeight = Microsoft.UI.Text.FontWeights.Medium;
            line.LineStackingStrategy = LineStackingStrategy.MaxHeight;
            line.HorizontalAlignment = HorizontalAlignment.Left;
        }
        brand.Children.Add(tagline);
        header.Children.Add(brand);
        var actions = new Grid { ColumnSpacing = 8, RowSpacing = 8, VerticalAlignment = VerticalAlignment.Center,
            HorizontalAlignment = HorizontalAlignment.Right };
        actions.ColumnDefinitions.Add(new()); actions.ColumnDefinitions.Add(new());
        actions.RowDefinitions.Add(new() { Height = GridLength.Auto });
        actions.RowDefinitions.Add(new() { Height = GridLength.Auto });
        // The WebView wraps the two groups, never the individual shortcuts.
        var quickActions = Row(8); quickActions.HorizontalAlignment = HorizontalAlignment.Right;
        foreach (var (label, icon, action) in new (string, string, Action)[]
        {
            (design.L("研究日历", "Research calendar"), "calendar", () => _ = OpenNativeAction("calendar")),
            (design.T("library.title"), "star", () => _ = OpenNativeAction("library")),
            (design.T("projects.search"), "search", OpenSearch),
            (design.T("sidebar.settings"), "gear", OpenSettings),
            (design.T("menu.docs"), "doc", OpenTutorials)
        })
        {
            var glyph = design.Icon(icon, 18); glyph.Opacity = .95;
            var button = new Button { Content = glyph };
            design.QuietButton(button);
            button.Padding = new Thickness(0); button.CornerRadius = new CornerRadius(10);
            button.BorderThickness = new Thickness(1); button.BorderBrush = new SolidColorBrush(Microsoft.UI.Colors.Transparent);
            button.Resources["ButtonBackgroundPointerOver"] = design.Brush("bg-elev");
            button.Resources["ButtonBorderBrushPointerOver"] = design.Brush("border");
            button.HorizontalContentAlignment = HorizontalAlignment.Center;
            button.Click += (_, _) => action();
            AutomationProperties.SetName(button, label); ToolTipService.SetToolTip(button, label);
            quickActions.Children.Add(button);
        }
        var projectActions = new Grid { ColumnSpacing = 8, HorizontalAlignment = HorizontalAlignment.Right };
        var assistantButton = HomeAction(design.T("assistant.open"), "sparkles", () => _ = OpenNativeAction("assistant"));
        AutomationProperties.SetAutomationId(assistantButton, "home-assistant"); projectActions.Children.Add(assistantButton);
        projectActions.Children.Add(HomeAction(design.T("projects.import"), "upload", () => _ = OpenNativeAction("import")));
        projectActions.Children.Add(HomeAction(design.T("projects.new"), "plus", () => _ = OpenNativeAction("create"), primary: true));
        var mainButtons = projectActions.Children.Cast<Button>().ToArray();
        for (var i = 0; i < mainButtons.Length; i++)
        {
            projectActions.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
            Grid.SetColumn(mainButtons[i], i); mainButtons[i].VerticalAlignment = VerticalAlignment.Center;
        }
        actions.Children.Add(quickActions); actions.Children.Add(projectActions);
        header.Children.Add(actions);
        page.Children.Add(header);
        if ((localError ?? model.Error) is { } error)
        {
            var banner = ErrorNotice(error, model.LastLoaded != null ? design.L("当前显示上次读取的数据。", "Showing the last loaded data.") : null);
            banner.Margin = new Thickness(0, 0, 0, 20);
            Grid.SetRow(banner, 1); page.Children.Add(banner);
        }
        var columns = new Grid { ColumnSpacing = HomeLayout.ColumnGap, RowSpacing = 26, VerticalAlignment = VerticalAlignment.Top };
        columns.ColumnDefinitions.Add(new() { Width = new GridLength(1, GridUnitType.Star) });
        columns.ColumnDefinitions.Add(new() { Width = new GridLength(1, GridUnitType.Star) });
        columns.RowDefinitions.Add(new() { Height = GridLength.Auto });
        columns.RowDefinitions.Add(new() { Height = GridLength.Auto });
        var projects = Stack(10);
        var metadata = new Dictionary<string, (Border Slot, ProjectSummary Project)>();
        foreach (var project in model.HomeProjects)
        {
            var exporting = projectExport?.Locks(project.Id) == true;
            var content = Stack(2);
            var nameRow = new Grid { ColumnSpacing = 12 };
            nameRow.ColumnDefinitions.Add(new() { Width = new GridLength(1, GridUnitType.Star) });
            nameRow.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
            var projectTitle = SingleLine(project.Name, 14); projectTitle.FontWeight = Microsoft.UI.Text.FontWeights.SemiBold;
            nameRow.Children.Add(projectTitle);
            var updated = SingleLine(NativeBrowserPresentation.RelativeTime(project.UpdatedAt, DateTimeOffset.Now, locale: design.Localization), 11, "text-faint");
            ToolTipService.SetToolTip(updated, NativeBrowserPresentation.ExactTime(project.UpdatedAt));
            Grid.SetColumn(updated, 1); nameRow.Children.Add(updated); content.Children.Add(nameRow);
            if (!string.IsNullOrWhiteSpace(project.WorkspaceDirectory)) content.Children.Add(HomeProjectPath(project.WorkspaceDirectory));
            var metaSlot = new Border { Child = ProjectCardMetadata(project) };
            metadata[project.Id] = (metaSlot, project); content.Children.Add(metaSlot);
            var cardRow = new Grid();
            cardRow.ColumnDefinitions.Add(new() { Width = new GridLength(1, GridUnitType.Star) });
            cardRow.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
            var open = ContentButton(content, () => _ = model.OpenProjectAsync(project.Id), $"project-{project.Id}", project.Name);
            open.Padding = new Thickness(18, 16, 8, 16);
            open.IsEnabled = !exporting;
            cardRow.Children.Add(open);
            var options = Row(2); options.VerticalAlignment = VerticalAlignment.Center; options.Margin = new Thickness(0, 0, 10, 0);
            Button CardAction(string label, string icon, Action action, bool selected = false)
            {
                var button = ActionButton(label, icon, action, quiet: true);
                var image = ((StackPanel)button.Content).Children.OfType<Image>().First();
                var idle = design.Icon(icon, 16, design.ColorText(selected ? "clay" : "text-faint")).Source;
                var hover = design.Icon(icon, 16, design.ColorText("clay")).Source;
                image.Source = idle;
                button.PointerEntered += (_, _) => image.Source = hover;
                button.PointerExited += (_, _) => image.Source = idle;
                return button;
            }
            var star = CardAction(project.Starred ? design.T("projects.unstar") : design.T("projects.star"), project.Starred ? "star-filled" : "star", () => _ = model.SetStarredAsync(project.Id, !project.Starred), project.Starred);
            star.IsEnabled = !model.Loading; options.Children.Add(star);
            options.Children.Add(CardAction(design.T("projects.settings"), "gear", () => OpenSettingsSection("project", project.Id)));
            var syncActions = HomeProjectSync.Actions(project, syncBackendConfigured);
            if (syncActions.Sync)
            {
                var sync = CardAction(design.T("projects.sync.now"), "sync", () => _ = RunHomeSyncAsync(project.Id));
                sync.IsEnabled = !homeSync.Busy(project.Id);
                AutomationProperties.SetAutomationId(sync, "project-sync-" + project.Id); options.Children.Add(sync);
                sync.Loaded += (_, _) => { if (restoreSyncFocus == project.Id) { restoreSyncFocus = null; sync.Focus(FocusState.Programmatic); } };
            }
            if (syncActions.Code)
            {
                var code = CardAction(design.T("projects.sync.copy_code"), "link", () => _ = CopyProjectCodeAsync(project.Id));
                AutomationProperties.SetAutomationId(code, "project-code-" + project.Id); options.Children.Add(code);
            }
            var export = CardAction(design.T("projects.export"), "download", () => _ = OpenProjectExportAsync(project));
            AutomationProperties.SetAutomationId(export, "project-export-" + project.Id); options.Children.Add(export);
            export.IsEnabled = projectExport?.Busy != true && projectImport?.Busy != true;
            export.Loaded += (_, _) => { if (restoreExportFocus == project.Id) { restoreExportFocus = null; export.Focus(FocusState.Programmatic); } };
            var newWindow = CardAction(design.T("projects.new_window"), "expand", () => ((App)Application.Current).OpenWindow(model.DatabasePath, project.Id));
            AutomationProperties.SetAutomationId(newWindow, "project-window-" + project.Id); options.Children.Add(newWindow);
            var delete = CardAction(design.T("projects.delete"), "close", () => _ = OpenProjectDeleteAsync(project));
            AutomationProperties.SetAutomationId(delete, "project-delete-" + project.Id); options.Children.Add(delete);
            delete.IsEnabled = projectExport?.Busy != true && projectImport?.Busy != true;
            delete.Loaded += (_, _) => { if (restoreDeleteFocus == project.Id) { restoreDeleteFocus = null; delete.Focus(FocusState.Programmatic); } };
            foreach (var button in options.Children.OfType<Button>())
            {
                button.Width = button.Height = 28; button.Padding = new Thickness(0); button.HorizontalContentAlignment = HorizontalAlignment.Center;
                button.IsEnabled &= !exporting; button.Opacity = button.IsEnabled ? .58 : .35;
                if (exporting) ToolTipService.SetToolTip(button, design.T("projects.transfer.export_locked"));
            }
            if (exporting) content.Children.Add(Text(design.T("projects.transfer.export_locked_badge"), 11, "clay-strong"));
            Grid.SetColumn(options, 1); cardRow.Children.Add(options);
            var card = Card(cardRow, 0); card.CornerRadius = new CornerRadius(12);
            bool hovered = false, focused = false;
            void UpdateActions()
            {
                foreach (var button in options.Children.OfType<Button>())
                    button.Opacity = button.IsEnabled ? button == star || hovered || focused ? 1 : .58 : .35;
            }
            card.PointerEntered += (_, _) => { hovered = true; UpdateActions(); };
            card.PointerExited += (_, _) => { hovered = false; UpdateActions(); };
            card.GotFocus += (_, _) => { focused = true; UpdateActions(); };
            card.LostFocus += (_, _) => { focused = false; UpdateActions(); };
            UpdateActions();
            var menu = new MenuFlyout();
            var projectSettings = new MenuFlyoutItem { Text = design.T("projects.settings"), IsEnabled = !exporting };
            projectSettings.Click += (_, _) => OpenSettingsSection("project", project.Id);
            menu.Items.Add(projectSettings);
            var reveal = new MenuFlyoutItem { Text = design.L("在资源管理器中显示", "Show in File Explorer"), IsEnabled = Directory.Exists(project.WorkspaceDirectory) };
            reveal.Click += (_, _) => Reveal(project);
            menu.Items.Add(reveal); Register(menu); card.ContextFlyout = menu;
            projects.Children.Add(card);
        }
        projects.Children.Insert(model.HomeProjects.TakeWhile(project => project.Starred).Count(), ExampleProjectCard());
        if (model.Projects.Count == 0 && (model.Loading || model.Error != null)) projects.Children.Add(Card(Text(model.Loading ? design.L("正在读取本地项目…", "Loading local projects…") : design.L("项目尚未读取成功，请刷新重试。", "Projects could not be loaded. Refresh to try again."), 14, "text-muted")));
        columns.Children.Add(HomeSection(design.T("projects.title"), projects));
        var recent = Stack(10);
        var savedActivity = Text(design.L("实时状态不可用，当前显示已保存的会话状态。", "Live status is unavailable. Showing saved session activity."), 12, "text-faint");
        AutomationProperties.SetLiveSetting(savedActivity, Microsoft.UI.Xaml.Automation.Peers.AutomationLiveSetting.Polite);
        var recentRows = new List<((string Project, string Id) Key, Action<BrowserSession> Update)>();
        void FillRecent()
        {
            var rows = model.HomeRecentSessions;
            savedActivity.Visibility = model.HomeActivitySavedOnly && rows.Count > 0 ? Visibility.Visible : Visibility.Collapsed;
            if (recentRows.Select(r => r.Key).SequenceEqual(rows.Select(s => (s.ProjectId, s.Id))))
            {
                for (var i = 0; i < rows.Count; i++) recentRows[i].Update(rows[i]);
                return;
            }
            recent.Children.Clear(); recentRows.Clear();
            foreach (var session in rows)
            {
                var card = RecentSessionCard(session, design, () => _ = model.OpenProjectAsync(session.ProjectId, session.Id),
                    projectExport?.Locks(session.ProjectId) == true, out var update);
                recentRows.Add(((session.ProjectId,session.Id), update)); recent.Children.Add(card);
            }
        }
        FillRecent();
        updateHomeRows = () =>
        {
            foreach (var p in model.HomeProjects)
                if (metadata.TryGetValue(p.Id, out var old) && old.Project != p)
                {
                    old.Slot.Child = ProjectCardMetadata(p);
                    metadata[p.Id] = (old.Slot, p);
                }
            FillRecent();
        };
        if (model.RecentSessions.Count == 0 && (model.Loading || model.LastLoaded == null && model.Error != null))
            recent.Children.Add(Text(model.Loading ? design.L("正在读取最近会话…", "Loading recent sessions…") : design.L("最近会话尚未读取成功，请刷新重试。", "Recent sessions could not be loaded. Refresh to try again."), 13, "text-faint"));
        var recentContent = Stack(8); recentContent.Children.Add(recent); recentContent.Children.Add(savedActivity);
        var recentSection = HomeSection(design.T("projects.recent"), recentContent);
        columns.Children.Add(recentSection); Grid.SetRow(columns, 2); page.Children.Add(columns);
        var footer = HomeFooter(); footer.Margin = new Thickness(0, 40, 0, 0);
        Grid.SetRow(footer, 3); page.Children.Add(footer);
        scroll.Content = page;

        void ArrangeHome(double width, double height)
        {
            var unconstrained = new Windows.Foundation.Size(double.PositiveInfinity, double.PositiveInfinity);
            // CSS rounds a 1px border down to whole device pixels. At 150%
            // scale the border is 2/3 DIP; rounding up on six edges can make
            // the whole toolbar wrap even though all of the content fits.
            var scale = scroll.XamlRoot?.RasterizationScale ?? 1;
            var border = new Thickness(Math.Max(1, Math.Floor(scale)) / scale);
            foreach (var button in mainButtons)
            {
                button.BorderThickness = border;
                button.Measure(unconstrained);
            }
            var natural = mainButtons.Select(b => b.DesiredSize.Width).ToArray();
            var chrome = mainButtons.Select(b => b.Padding.Left + b.Padding.Right + b.BorderThickness.Left + b.BorderThickness.Right).ToArray();
            var mainActionsWidth = natural.Sum() + projectActions.ColumnSpacing * (mainButtons.Length - 1);
            var home = HomeLayout.ForWidth(width, design.Typography.UiSize / 14, mainActionsWidth);
            var buttonWidths = HomeLayout.MainButtonWidths(natural, chrome, home.ActionWidth);
            for (var i = 0; i < buttonWidths.Length; i++)
                projectActions.ColumnDefinitions[i].Width = mainActionsWidth <= home.ActionWidth
                    ? GridLength.Auto : new GridLength(buttonWidths[i]);
            foreach (var line in tagline.Children.OfType<TextBlock>())
            {
                line.FontSize = HomeLayout.TaglineSize(width);
                line.LineHeight = line.FontSize * 1.65;
                var brandWidth = home.ActionsBelowBrand ? home.ContentWidth : home.BrandWidth;
                var available = brandWidth - (home.StackBrand ? 0 : HomeLayout.WordmarkWidth + HomeLayout.HeaderGap);
                line.MaxWidth = double.PositiveInfinity;
                line.MaxWidth = HomeLayout.BalancedTaglineWidth(available, w =>
                {
                    line.Measure(new Windows.Foundation.Size(w, double.PositiveInfinity));
                    return line.DesiredSize.Height;
                });
            }
            foreach (var button in quickActions.Children.OfType<Button>())
                button.Width = button.Height = HomeLayout.ShortcutSize(width);
            // A stretched, MaxWidth-constrained ScrollViewer child can be arranged
            // off-center after resize. Give the centered page its measured budget.
            page.Width = home.ContentWidth;
            var inset = home.CenteredInset(width);
            page.Margin = new Thickness(inset, home.TopPadding, inset, 40);
            page.MinHeight = Math.Max(0, height - home.TopPadding - 40);
            header.ColumnDefinitions[0].Width = home.ActionsBelowBrand ? new GridLength(1, GridUnitType.Star) : new GridLength(home.BrandWidth);
            header.ColumnDefinitions[1].Width = home.ActionsBelowBrand ? new GridLength(0) : new GridLength(1, GridUnitType.Star);
            Grid.SetColumn(actions, home.ActionsBelowBrand ? 0 : 1); Grid.SetRow(actions, home.ActionsBelowBrand ? 1 : 0);
            brand.ColumnDefinitions[0].Width = home.StackBrand ? new GridLength(1, GridUnitType.Star) : new GridLength(HomeLayout.WordmarkWidth);
            brand.ColumnDefinitions[1].Width = home.StackBrand ? new GridLength(0) : new GridLength(1, GridUnitType.Star);
            Grid.SetColumn(tagline, home.StackBrand ? 0 : 1); Grid.SetRow(tagline, home.StackBrand ? 1 : 0);
            // Collapsed grid tracks must also lose their gaps.
            header.ColumnSpacing = home.ActionsBelowBrand ? 0 : HomeLayout.HeaderGap;
            header.RowSpacing = home.ActionsBelowBrand ? HomeLayout.HeaderGap : 0;
            brand.ColumnSpacing = home.StackBrand ? 0 : HomeLayout.HeaderGap;
            brand.RowSpacing = home.StackBrand ? 12 : 0;
            actions.MaxWidth = home.ActionWidth;
            actions.ColumnDefinitions[0].Width = home.StackActions ? new GridLength(1, GridUnitType.Star) : GridLength.Auto;
            actions.ColumnDefinitions[1].Width = home.StackActions ? new GridLength(0) : new GridLength(1, GridUnitType.Star);
            actions.ColumnSpacing = home.StackActions ? 0 : 8;
            actions.RowSpacing = home.StackActions ? 8 : 0;
            Grid.SetColumn(projectActions, home.StackActions ? 0 : 1); Grid.SetRow(projectActions, home.StackActions ? 1 : 0);
            columns.ColumnDefinitions[1].Width = home.StackColumns ? new GridLength(0) : new GridLength(1, GridUnitType.Star);
            columns.ColumnSpacing = home.StackColumns ? 0 : HomeLayout.ColumnGap;
            columns.RowSpacing = home.StackColumns ? 26 : 0;
            Grid.SetColumn(recentSection, home.StackColumns ? 0 : 1); Grid.SetRow(recentSection, home.StackColumns ? 1 : 0);
        }
        scroll.SizeChanged += (_, e) => ArrangeHome(e.NewSize.Width, e.NewSize.Height);
        scroll.Loaded += (_, _) => ArrangeHome(scroll.ActualWidth, scroll.ActualHeight);
        ArrangeHome(root.ActualWidth > 0 ? root.ActualWidth : 800, root.ActualHeight);
        var backdrop = new NativeHomeBackdrop(design, scroll);
        if (homeSyncNotice != null) backdrop.Children.Add(HomeSyncNotice(homeSyncNotice));
        return backdrop;
    }

    private Grid HomeProjectPath(string fullPath)
    {
        var slot = new Grid();
        var text = SingleLine(fullPath, 11, "text-faint");
        text.FontFamily = design.CodeFont;
        text.TextTrimming = TextTrimming.Clip;
        AutomationProperties.SetName(text, fullPath);
        // A detached text probe uses different DPI/font metrics. Measure in the
        // same XamlRoot, without letting the probe affect the card's layout.
        var canvas = new Canvas { Height = 0, Opacity = 0, IsHitTestVisible = false };
        var probe = new TextBlock { TextWrapping = TextWrapping.NoWrap };
        AutomationProperties.SetAccessibilityView(probe, Microsoft.UI.Xaml.Automation.Peers.AccessibilityView.Raw);
        canvas.Children.Add(probe); slot.Children.Add(canvas); slot.Children.Add(text);
        void Fit()
        {
            if (slot.ActualWidth <= 0) return;
            probe.FontFamily = text.FontFamily; probe.FontSize = text.FontSize;
            probe.FontWeight = text.FontWeight; probe.CharacterSpacing = text.CharacterSpacing;
            text.Text = NativeHomePresentation.FitPath(fullPath, slot.ActualWidth, candidate =>
            {
                probe.Text = candidate;
                probe.Measure(new Windows.Foundation.Size(double.PositiveInfinity, double.PositiveInfinity));
                return probe.DesiredSize.Width;
            });
        }
        slot.SizeChanged += (_, _) => Fit();
        slot.Loaded += (_, _) => Fit();
        text.RegisterPropertyChangedCallback(TextBlock.FontSizeProperty, (_, _) => Fit());
        text.RegisterPropertyChangedCallback(TextBlock.FontFamilyProperty, (_, _) => Fit());
        return slot;
    }

    private FrameworkElement HomeSection(string title, UIElement items)
    {
        var section = Stack(14); section.Margin = new Thickness(0, design.FontSize(18) * .83, 0, 0);
        var heading = Text(title, 18); heading.FontWeight = Microsoft.UI.Text.FontWeights.Bold;
        section.Children.Add(heading); section.Children.Add(items); return section;
    }

    // The same card renderer is usable in the database-free visual review host.
    internal static Border RecentSessionCard(BrowserSession session, WispDesign design, Action openSession, bool locked)
        => RecentSessionCard(session, design, openSession, locked, out _);

    private static Border RecentSessionCard(BrowserSession session, WispDesign design, Action openSession, bool locked, out Action<BrowserSession> update)
    {
        var label = new Grid { ColumnSpacing = 8, HorizontalAlignment = HorizontalAlignment.Left }; label.ColumnDefinitions.Add(new()); label.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
        var title = design.Text(NativeBrowserPresentation.SessionTitle(session.Title, design.Localization), 14);
        title.TextWrapping = TextWrapping.NoWrap; title.TextTrimming = TextTrimming.CharacterEllipsis;
        title.FontWeight = Microsoft.UI.Text.FontWeights.SemiBold; title.VerticalAlignment = VerticalAlignment.Center; label.Children.Add(title);
        var caption = design.Text("", 11); caption.TextWrapping = TextWrapping.NoWrap;
        caption.FontWeight = Microsoft.UI.Text.FontWeights.SemiBold;
        var badge = new Border { Child = caption,
            CornerRadius = new CornerRadius(999), Padding = new Thickness(8, 2, 8, 2), VerticalAlignment = VerticalAlignment.Center };
        Grid.SetColumn(badge, 1); label.Children.Add(badge);
        var open = new Button { Content = label, IsEnabled = !locked, Padding = new Thickness(18, 16, 18, 16), Background = new SolidColorBrush(Microsoft.UI.Colors.Transparent),
            BorderThickness = new Thickness(0), HorizontalAlignment = HorizontalAlignment.Stretch, HorizontalContentAlignment = HorizontalAlignment.Stretch };
        open.Click += (_, _) => openSession(); AutomationProperties.SetAutomationId(open, "recent-" + session.Id);
        update = row =>
        {
            var state = NativeHomePresentation.Status(row.Status);
            title.Text = NativeBrowserPresentation.SessionTitle(row.Title, design.Localization);
            caption.Text = design.T("sess_status." + state);
            caption.Foreground = design.Brush(state == "running" ? "clay" : state == "needs_you" ? "info" : "text-faint");
            badge.Background = design.Brush(state == "needs_you" ? "info-soft" : state == "running" ? "clay-soft" : "bg-sunken");
            AutomationProperties.SetName(open, title.Text + " · " + caption.Text); ToolTipService.SetToolTip(open, title.Text);
        };
        update(session);
        var card = design.Card(open); card.Padding = new Thickness(0); card.CornerRadius = new CornerRadius(12); return card;
    }

    private Button HomeAction(string label, string icon, Action action, bool primary = false)
    {
        var content = new Grid { ColumnSpacing = primary ? 2 : 7 };
        content.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
        content.ColumnDefinitions.Add(new() { Width = new GridLength(1, GridUnitType.Star) });
        var glyph = design.Icon(icon, 18, primary ? design.ColorText("on-clay") : null);
        if (primary)
        {
            // The reference plus is inline type: a ~12px advance at 14px UI
            // size, not an 18px icon slot. Keep the shared vector centered.
            glyph.Width = design.FontSize(12); glyph.Height = design.FontSize(16);
            glyph.Stretch = Stretch.UniformToFill;
        }
        content.Children.Add(glyph);
        var text = Text(label, 13.5);
        text.TextWrapping = TextWrapping.Wrap; text.TextAlignment = TextAlignment.Center;
        if (primary) { text.Foreground = design.Brush("on-clay"); text.FontWeight = Microsoft.UI.Text.FontWeights.SemiBold; }
        Grid.SetColumn(text, 1); content.Children.Add(text);
        var button = new Button { Content = content, HorizontalAlignment = HorizontalAlignment.Stretch,
            HorizontalContentAlignment = HorizontalAlignment.Stretch };
        design.ActionButton(button, primary); button.CornerRadius = new CornerRadius(10); button.MinHeight = 0;
        button.Padding = primary ? new Thickness(16, 9, 16, 9) : new Thickness(12, 8, 12, 8);
        if (!primary)
        {
            button.Background = new SolidColorBrush(Microsoft.UI.Colors.Transparent);
            button.Resources["ButtonBackgroundPointerOver"] = design.Brush("bg-sunken");
        }
        button.Click += (_, _) => action();
        AutomationProperties.SetName(button, label); ToolTipService.SetToolTip(button, label);
        return button;
    }

    private FrameworkElement HomeFooter()
    {
        var footer = new Grid { ColumnSpacing = 12 };
        footer.ColumnDefinitions.Add(new() { Width = new GridLength(38) });
        footer.ColumnDefinitions.Add(new() { Width = new GridLength(1, GridUnitType.Star) });
        footer.ColumnDefinitions.Add(new() { Width = new GridLength(38) });
        var utilities = ActionButton(design.L("更多首页选项", "More home options"), "more", () => { }, quiet: true);
        var menu = new MenuFlyout();
        var refresh = new MenuFlyoutItem { Text = design.L("刷新", "Refresh"), IsEnabled = !model.Loading };
        refresh.Click += (_, _) => { localError = null; _ = model.RefreshAsync(); }; menu.Items.Add(refresh);
        var database = new MenuFlyoutItem { Text = design.L("选择数据库", "Choose database") };
        database.Click += (_, _) => ChooseDatabase(); menu.Items.Add(database);
        menu.Items.Add(new MenuFlyoutSeparator());
        foreach (var (label, value) in new[] { (design.L("跟随系统", "System"), "system"), (design.L("浅色", "Light"), "light"), (design.L("深色", "Dark"), "dark") })
        {
            var item = new ToggleMenuFlyoutItem { Text = design.L("外观：", "Appearance: ") + label, IsChecked = settings.Appearance == value, IsEnabled = !changingAppearance };
            item.Click += async (_, _) => await ChangeAppearanceAsync(value); menu.Items.Add(item);
        }
        if (model.LastLoaded is { } time) { menu.Items.Add(new MenuFlyoutSeparator()); menu.Items.Add(new MenuFlyoutItem { Text = design.L($"更新于 {time:HH:mm:ss}", $"Updated {time:HH:mm:ss}"), IsEnabled = false }); }
        Register(menu); utilities.Flyout = menu; footer.Children.Add(utilities);
        var support = new TextBlock { TextWrapping = TextWrapping.Wrap, TextAlignment = TextAlignment.Center,
            VerticalAlignment = VerticalAlignment.Bottom, Foreground = design.Brush("text-faint") };
        design.BindTypography(support, 13);
        support.Inlines.Add(new Microsoft.UI.Xaml.Documents.Run { Text = design.T("projects.star_hint") + " " });
        var link = new Microsoft.UI.Xaml.Documents.Hyperlink { NavigateUri = new Uri("https://github.com/xuzhougeng/wisp-science"), Foreground = design.Brush("clay") };
        link.Inlines.Add(new Microsoft.UI.Xaml.Documents.Run { Text = design.T("projects.star_link") });
        support.Inlines.Add(link);
        Grid.SetColumn(support, 1); footer.Children.Add(support);
        return footer;
    }

    private FrameworkElement Workspace(ProjectSummary project)
    {
        var shell = new Grid();
        workspaceShell = shell;
        shell.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
        shell.ColumnDefinitions.Add(new() { Width = new GridLength(1, GridUnitType.Star) });
        shell.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
        var sidebar = new ContentControl { Content = Sidebar(project), IsTabStop = false,
            HorizontalContentAlignment = HorizontalAlignment.Stretch, VerticalContentAlignment = VerticalAlignment.Stretch };
        var main = new Grid();
        workspaceMain = main;
        main.RowDefinitions.Add(new() { Height = GridLength.Auto });
        main.RowDefinitions.Add(new() { Height = new GridLength(1, GridUnitType.Star) });
        main.RowDefinitions.Add(new() { Height = GridLength.Auto });
        main.RowDefinitions.Add(new() { Height = GridLength.Auto });
        var toolbar = workspaceToolbar = new Grid { Padding = new Thickness(14, 9, 14, 9),
            BorderThickness = new Thickness(0, 0, 0, 1), BorderBrush = design.Brush("border"), Background = design.Brush("bg-app") };
        toolbar.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
        toolbar.ColumnDefinitions.Add(new() { Width = new GridLength(1, GridUnitType.Star) });
        toolbar.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
        var navigation = Row(4);
        navigation.Margin = new Thickness(0, 0, 10, 0);
        navigation.Children.Add(ActionButton(design.T("sidebar.back_projects"), "arrow-left", model.GoHome));
        navigation.Children.Add(ActionButton(design.T("sidebar.show"), "chevron-right", () =>
        {
            sidebarVisible = true;
            Render();
        }));
        toolbar.Children.Add(navigation);
        var sessionTitle = NativeBrowserPresentation.SessionTitle(model.ActiveSession?.Title
            ?? design.T("center.new_session"), design.Localization);
        var tabs = centerFiles?.Header ?? new NativeCenterTabs(design);
        if (centerFiles != null) centerFiles.SetConversationTitle(sessionTitle);
        else tabs.Present(sessionTitle, [], null, () => { if (projectPage != null) CloseProjectPage(); }, _ => { }, _ => { });
        Grid.SetColumn(tabs, 1); toolbar.Children.Add(tabs);
        var tools = Row(2);
        var sessionReady = model.ActiveSessionId != null;
        Button ToolbarButton(string label, string icon, Action? action)
        {
            var button = ActionButton(label, icon, action, quiet: true);
            button.Width = button.Height = button.MinHeight = 30; button.MinWidth = 0;
            button.Padding = new(0); button.CornerRadius = new(8); button.HorizontalContentAlignment = HorizontalAlignment.Center;
            return button;
        }
        var outline = ToolbarButton(design.L("会话大纲", "Conversation outline"), "list", sessionReady ? () => _ = OpenSheet("outline") : null);
        tools.Children.Add(outline);
        tools.Children.Add(ToolbarButton(design.T("share.topbar"), "share", sessionReady ? () => _ = OpenSheet("share") : null));
        tools.Children.Add(ToolbarButton(design.T("trajectory.topbar"), "timeline", sessionReady ? () => _ = OpenSheet("trajectory") : null));
        tools.Children.Add(ToolbarButton(design.L("研究归档", "Archive research"), "archive", sessionReady ? () => _ = OpenSheet("archive") : null));
        var inboxLabel = design.L("待查看", "Need you") + (inbox?.Entries.Length > 0 ? $" {inbox.Entries.Length}" : "");
        tools.Children.Add(ToolbarButton(inboxLabel, "bell", () => _ = OpenSheet("inbox")));
        tools.Children.Add(ToolbarButton(design.T("contexts.open_terminal"), "terminal", sessionReady ? ToggleTerminal : null));
        var mainTools = tools.Children.ToArray();
        var viewToggle = sessionReady && conversation != null
            ? new NativeTranscriptViewToggle(conversation.ModelContext, design, value => conversationPage?.SelectModelView(value)) : null;
        if (viewToggle != null) { viewToggle.Margin = new(0, 0, 7, 0); tools.Children.Insert(0, viewToggle); }
        var overflow = ToolbarButton(design.L("更多会话操作", "More conversation actions"), "more", () => { });
        var overflowMenu = new MenuFlyout { Placement = FlyoutPlacementMode.BottomEdgeAlignedRight };
        var overflowStyle = new Style(typeof(MenuFlyoutPresenter));
        overflowStyle.Setters.Add(new Setter(FrameworkElement.MinWidthProperty, 200d));
        overflowStyle.Setters.Add(new Setter(Control.PaddingProperty, new Thickness(6)));
        overflowStyle.Setters.Add(new Setter(Control.BackgroundProperty, design.Brush("bg-elev")));
        overflowStyle.Setters.Add(new Setter(Control.BorderBrushProperty, design.Brush("border-strong")));
        overflowStyle.Setters.Add(new Setter(Control.BorderThicknessProperty, new Thickness(1)));
        overflowStyle.Setters.Add(new Setter(Control.CornerRadiusProperty, new CornerRadius(12)));
        overflowMenu.MenuFlyoutPresenterStyle = overflowStyle;
        if (viewToggle != null && conversation is { } owner)
        {
            foreach (var (key, icon, value) in new[] { ("chat.view_full", "history", false), ("chat.view_model", "eye", true) })
            {
                var item = new ToggleMenuFlyoutItem { Text = design.T(key), Icon = new ImageIcon { Source = design.Icon(icon, 16).Source } };
                item.Click += (_, _) => conversationPage?.SelectModelView(value);
                overflowMenu.Opened += (_, _) => { item.IsChecked = owner.ModelContext.Active == value; item.IsEnabled = !value || owner.ModelContext.Available; };
                overflowMenu.Items.Add(item);
            }
            overflowMenu.Items.Add(new MenuFlyoutSeparator());
        }
        foreach (var (label, icon, action) in new (string, string, Action)[] {
            (design.L("会话大纲", "Conversation outline"), "list", () => _ = OpenSheet("outline")), (design.T("share.topbar"), "share", () => _ = OpenSheet("share")),
            (design.T("trajectory.topbar"), "timeline", () => _ = OpenSheet("trajectory")), (design.L("研究归档", "Archive research"), "archive", () => _ = OpenSheet("archive")),
            (inboxLabel, "bell", () => _ = OpenSheet("inbox")), (design.T("contexts.open_terminal"), "terminal", ToggleTerminal) })
        {
            if (icon == "list" && !sessionReady) continue;
            var item = new MenuFlyoutItem { Text = label, Icon = new ImageIcon { Source = design.Icon(icon, 16).Source },
                IsEnabled = icon == "bell" || sessionReady, MinHeight = 34, Padding = new(10, 0, 10, 0) };
            design.BindTypography(item, size: 13);
            item.Click += (_, _) => action(); overflowMenu.Items.Add(item);
        }
        Register(overflowMenu); overflow.Flyout = overflowMenu; tools.Children.Add(overflow);
        tools.Children.Add(ToolbarButton(design.T("center.toggle_panel"), "panel", TogglePanel));
        void FitToolbar(double width)
        {
            var compact = PreviewLayout.CompactConversationToolbar(width);
            foreach (var tool in mainTools) tool.Visibility = compact ? Visibility.Collapsed : Visibility.Visible;
            if (viewToggle != null) viewToggle.Visibility = compact ? Visibility.Collapsed : Visibility.Visible;
            if (!sessionReady) outline.Visibility = Visibility.Collapsed;
            overflow.Visibility = compact ? Visibility.Visible : Visibility.Collapsed;
            tabs.SetPaneWidth(width);
        }
        FitToolbar(PreviewLayout.Workspace(root.ActualWidth, sidebarVisible, panelVisible, settings.PanelWidth).ContentWidth);
        toolbar.SizeChanged += (_, e) => FitToolbar(e.NewSize.Width);
        var toolGroupColor = design.Brush("bg-elev").Color; toolGroupColor.A = (byte)Math.Round(toolGroupColor.A * .7);
        var toolGroup = new Border { Child = tools, Padding = new(2), CornerRadius = new(8), BorderThickness = new(1),
            BorderBrush = design.Brush("border"), Background = new SolidColorBrush(toolGroupColor), HorizontalAlignment = HorizontalAlignment.Right,
            Margin = new Thickness(10, 0, 0, 0) };
        Grid.SetColumn(toolGroup, 2); toolbar.Children.Add(toolGroup); main.Children.Add(toolbar);
        if (projectPage != null) toolbar.Visibility = Visibility.Collapsed;
        var messages = Stack(8); messages.MaxWidth = PreviewLayout.ConversationMaxWidth; messages.Margin = new Thickness(16);
        if ((localError ?? model.SessionError ?? model.Error) is { } error)
        {
            messages.Children.Add(ErrorNotice(error));
            messages.Children.Add(ActionButton("重新连接", "refresh", () => { localError = null; _ = EnsureConversationAsync(); }, true));
        }
        if (model.NextBeforeSeq != null)
        {
            var older = ActionButton("加载更早的消息", "clock", () => _ = model.OpenSessionAsync(model.ActiveSessionId!, older: true), true);
            older.IsEnabled = !model.TranscriptLoading; messages.Children.Add(older);
        }
        savedChatRow = null;
        foreach (var group in TranscriptPresentation.Groups(model.Messages))
        {
            if (group.IsProcess)
            {
                var process = Stack(4);
                var hasMatch = false;
                foreach (var item in group.Messages)
                {
                    var tool = TranscriptView.Create(item, design, inlineTools: true); process.Children.Add(tool);
                    if (savedChatSequence == item.Sequence) { savedChatRow = tool; hasMatch = true; }
                }
                messages.Children.Add(new Expander { Header = $"查看工具过程 · {group.Messages.Count} 条", Content = process, IsExpanded = hasMatch,
                    HorizontalAlignment = HorizontalAlignment.Stretch, HorizontalContentAlignment = HorizontalAlignment.Stretch });
                continue;
            }
            var message = group.Messages[0];
            var entry = Stack(8);
            if (savedChatSequence == message.Sequence) savedChatRow = entry;
            if (message.Role != "tool" || message.ToolName == "attempt_completion")
                entry.Children.Add(Text(message.Role == "user" ? "你" : "Wisp Science", 12, "text-muted"));
            entry.Children.Add(TranscriptView.Create(message, design, revealTool: savedChatSequence == message.Sequence));
            var card = Card(entry, 10); card.Background = design.Brush(message.Role == "user" ? "bg-sunken" : "bg-app");
            card.BorderThickness = new Thickness(0); messages.Children.Add(card);
        }
        if (model.SessionsLoading || model.TranscriptLoading) messages.Children.Add(new ProgressBar { IsIndeterminate = true, Width = 180 });
        else if (model.Messages.Count == 0 && model.SessionError == null) messages.Children.Add(Text(model.Sessions.Count == 0 ? "这个项目还没有会话" : "这个会话暂无消息", 14, "text-faint"));
        if (projectPage is UserControl projectControl)
        {
            if (projectControl.Parent is Panel parent) parent.Children.Remove(projectControl);
            Grid.SetRow(projectControl, 1); main.Children.Add(projectControl);
        }
        else if (centerFiles != null)
        {
            if (centerFiles.Parent is Panel previous) previous.Children.Remove(centerFiles);
            centerFiles.AttachConversation(conversationPage);
            Grid.SetRow(centerFiles, 1); main.Children.Add(centerFiles);
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
            bottom.Children.Add(Text(localError == null ? "正在连接桌面服务…连接后可继续对话。" : "当前为只读浏览，可查看历史消息。", 12, "text-muted"));
            Grid.SetRow(bottom, 3); main.Children.Add(bottom);
        }
        if (projectPage == null && terminalVisible && model.ActiveSessionId != null && terminalPage != null)
        {
            terminalPage.MaxHeight = PreviewLayout.TerminalMaxHeight(root.ActualHeight);
            if (terminalPage.Parent is Panel previous) previous.Children.Remove(terminalPage);
            Grid.SetRow(terminalPage, 2); main.Children.Add(terminalPage);
        }
        var mainSurface = new ContentControl { Content = main, IsTabStop = false,
            HorizontalContentAlignment = HorizontalAlignment.Stretch, VerticalContentAlignment = VerticalAlignment.Stretch };
        Grid.SetColumn(mainSurface, 1); shell.Children.Add(mainSurface);
        shell.Children.Add(sidebar);
        Grid? panelHost = null;
        Button? backdrop = null;
        Thumb? resize = null;
        if (projectPage == null && panelVisible && model.ActiveProjectId != null)
        {
            FrameworkElement panelContent;
            if (panelPage != null)
            {
                if (panelPage.Parent is Panel previous) previous.Children.Remove(panelPage);
                panelContent = panelPage;
            }
            else
            {
                var pending = Stack(12); pending.Margin = new Thickness(16);
                pending.Children.Add(Text(settings.PanelTab == "files" ? design.T("sidebar.files") : "工作区面板", 18));
                pending.Children.Add(Text(localError ?? "正在连接桌面服务…", 13));
                if (localError != null) pending.Children.Add(ActionButton("重新连接", "refresh", () => { localError = null; _ = EnsurePanelAndTerminalAsync(); }, true));
                pending.Children.Add(ActionButton("关闭面板", "close", TogglePanel, true));
                panelContent = pending;
            }
            backdrop = new Button { Background = new SolidColorBrush(Windows.UI.Color.FromArgb(41, 20, 20, 19)),
                BorderThickness = new Thickness(0), HorizontalAlignment = HorizontalAlignment.Stretch,
                VerticalAlignment = VerticalAlignment.Stretch, IsTabStop = false };
            // A dismiss surface must retain the same tint when the pointer moves over it.
            backdrop.Resources["ButtonBackgroundPointerOver"] = backdrop.Background;
            backdrop.Resources["ButtonBackgroundPressed"] = backdrop.Background;
            backdrop.Resources["ButtonBorderBrushPointerOver"] = new SolidColorBrush(Microsoft.UI.Colors.Transparent);
            backdrop.Resources["ButtonBorderBrushPressed"] = new SolidColorBrush(Microsoft.UI.Colors.Transparent);
            AutomationProperties.SetName(backdrop, "关闭侧面板");
            backdrop.Click += (_, _) => TogglePanel();
            Grid.SetColumnSpan(backdrop, 3); shell.Children.Add(backdrop);
            panelHost = new Grid { HorizontalAlignment = HorizontalAlignment.Right, Background = design.Brush("bg-elev"),
                BorderBrush = design.Brush("border"), BorderThickness = new Thickness(1, 0, 0, 0) };
            workspacePanelHost = panelHost;
            panelHost.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
            panelHost.ColumnDefinitions.Add(new() { Width = new GridLength(1, GridUnitType.Star) });
            resize = new Thumb { Width = 5, Background = new SolidColorBrush(Microsoft.UI.Colors.Transparent), IsTabStop = true };
            AutomationProperties.SetName(resize, "调整侧面板宽度，左右方向键调整");
            resize.DragStarted += (_, _) => settings.PanelWidth = panelHost.ActualWidth - PreviewLayout.PanelResizerWidth;
            resize.DragDelta += (_, e) => { settings.PanelWidth = PreviewLayout.ClampPanelWidth(settings.PanelWidth - e.HorizontalChange); SizeWorkspace(); };
            resize.KeyDown += (_, e) =>
            {
                if (e.Key is not (VirtualKey.Left or VirtualKey.Right)) return;
                settings.PanelWidth = PreviewLayout.ClampPanelWidth(panelHost.ActualWidth - PreviewLayout.PanelResizerWidth + (e.Key == VirtualKey.Left ? 24 : -24));
                SizeWorkspace(); e.Handled = true;
            };
            panelHost.Children.Add(resize);
            Grid.SetColumn(panelContent, 1); panelHost.Children.Add(panelContent);
            shell.Children.Add(panelHost);
        }
        shell.SizeChanged += (_, _) => SizeWorkspace();
        SizeWorkspace();
        return shell;

        void SizeWorkspace()
        {
            if (!ReferenceEquals(workspaceShell, shell)) return;
            var width = shell.ActualWidth > 0 ? shell.ActualWidth : root.ActualWidth > 0 ? root.ActualWidth : 1220;
            var columns = PreviewLayout.Workspace(width, sidebarVisible, panelHost != null, settings.PanelWidth);
            sidebar.Width = columns.SidebarWidth;
            sidebar.HorizontalAlignment = HorizontalAlignment.Left;
            sidebar.Visibility = columns.SidebarWidth > 0 ? Visibility.Visible : Visibility.Collapsed;
            mainSurface.IsEnabled = !columns.PanelOverlay;
            sidebar.IsEnabled = !columns.PanelOverlay;
            navigation.Visibility = columns.SidebarWidth > 0 ? Visibility.Collapsed : Visibility.Visible;
            workspacePanelOverlay = columns.PanelOverlay;
            UpdateSheetPresentation();
            if (panelHost == null) return;
            panelHost.Width = columns.PanelWidth;
            Grid.SetColumn(panelHost, columns.PanelOverlay ? 0 : 2);
            Grid.SetColumnSpan(panelHost, columns.PanelOverlay ? 3 : 1);
            backdrop!.Visibility = columns.PanelOverlay ? Visibility.Visible : Visibility.Collapsed;
            resize!.Visibility = columns.PanelOverlay ? Visibility.Collapsed : Visibility.Visible;
        }
    }

    private FrameworkElement Sidebar(ProjectSummary project)
    {
        var rail = layout.CompactWorkspace;
        var sidebar = new Grid { Width = rail ? PreviewLayout.SidebarRailWidth : PreviewLayout.SidebarWidth,
            Padding = rail ? new Thickness(6, 10, 6, 10) : new Thickness(10, 12, 10, 12),
            Background = design.Brush("bg-sunken"), BorderBrush = design.Brush("border"),
            BorderThickness = new Thickness(0, 0, 1, 0), RowSpacing = 6 };
        sidebar.RowDefinitions.Add(new() { Height = GridLength.Auto });
        sidebar.RowDefinitions.Add(new() { Height = new GridLength(1, GridUnitType.Star) });
        sidebar.RowDefinitions.Add(new() { Height = GridLength.Auto });
        var top = Stack(6);
        var back = ActionButton(design.T("sidebar.back_projects"), "arrow-left", model.GoHome, quiet: true);
        var switcher = ActionButton(project.Name, null, () => { }, quiet: true);
        Panel heading;
        Button? collapseButton = null;
        if (rail)
        {
            heading = Stack(6);
            heading.HorizontalAlignment = HorizontalAlignment.Center; heading.Margin = new Thickness(0, 2, 0, 4);
            back.Width = back.Height = 32; back.Padding = new Thickness(0); back.HorizontalContentAlignment = HorizontalAlignment.Center;
            switcher.Width = switcher.Height = 36; switcher.Padding = new Thickness(0); switcher.HorizontalContentAlignment = HorizontalAlignment.Center;
            switcher.Content = new Image { Width = 18, Height = 18, Source = new Microsoft.UI.Xaml.Media.Imaging.SvgImageSource(
                new Uri(Path.Combine(AppContext.BaseDirectory, "Assets", "logo.svg"))) };
        }
        else
        {
            var head = new Grid { Padding = new Thickness(4,4,4,6), ColumnSpacing = 4 };
            head.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
            head.ColumnDefinitions.Add(new() { Width = new GridLength(1, GridUnitType.Star) });
            head.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
            back.Width = back.Height = 28; back.Padding = new(0); back.HorizontalContentAlignment = HorizontalAlignment.Center;
            var name = SingleLine(project.Name, 14); name.FontWeight = Microsoft.UI.Text.FontWeights.SemiBold;
            switcher.Content = name; switcher.HorizontalAlignment = HorizontalAlignment.Stretch;
            switcher.Padding = new(8,5,8,5); ToolTipService.SetToolTip(switcher, project.Name);
            Grid.SetColumn(switcher, 1);
            var collapse = ActionButton(design.T("sidebar.collapse"), "chevron-left", () => { sidebarVisible = false; Render(); }, quiet: true);
            collapse.Width = collapse.Height = 28; collapse.Padding = new(0); collapse.HorizontalContentAlignment = HorizontalAlignment.Center;
            Grid.SetColumn(collapse, 2); collapseButton = collapse; heading = head;
        }
        heading.Children.Add(back);
        AutomationProperties.SetName(switcher, design.L("切换项目", "Switch project"));
        var menu = new MenuFlyout();
        var projectSettings = new MenuFlyoutItem { Text = design.T("projects.settings") };
        projectSettings.Click += (_, _) => OpenSettingsSection("project", project.Id);
        menu.Items.Add(projectSettings);
        var importSession = new MenuFlyoutItem { Text = design.L("导入会话归档…", "Import conversation archive…") };
        importSession.Click += (_, _) => _ = OpenNativeAction("import-session");
        menu.Items.Add(importSession);
        var importExternal = new MenuFlyoutItem { Text = design.L("导入 Codex / Claude 会话…", "Import Codex / Claude conversations…") };
        importExternal.Click += (_, _) => _ = OpenNativeAction("import-cli");
        menu.Items.Add(importExternal);
        menu.Items.Add(new MenuFlyoutSeparator());
        foreach (var item in model.Projects)
        {
            var option = new MenuFlyoutItem { Text = item.Name };
            option.Click += async (_, _) => await model.OpenProjectAsync(item.Id);
            menu.Items.Add(option);
        }
        menu.Items.Add(new MenuFlyoutSeparator());
        var palette = new MenuFlyoutItem { Text = design.L("命令面板", "Command palette"), KeyboardAcceleratorTextOverride = "Ctrl+Shift+P" };
        palette.Click += (_, _) => OpenSearch(true); menu.Items.Add(palette);
        var reveal = new MenuFlyoutItem { Text = design.L("在资源管理器中显示", "Show in File Explorer") }; reveal.Click += (_, _) => Reveal(project); menu.Items.Add(reveal);
        var refresh = new MenuFlyoutItem { Text = design.L("刷新", "Refresh"), IsEnabled = !model.Loading }; refresh.Click += (_, _) => _ = model.RefreshAsync(); menu.Items.Add(refresh);
        var database = new MenuFlyoutItem { Text = design.L("选择数据库", "Choose database") }; database.Click += (_, _) => ChooseDatabase(); menu.Items.Add(database);
        foreach (var (label, value) in new[] { (design.L("跟随系统", "System"), "system"), (design.L("浅色", "Light"), "light"), (design.L("深色", "Dark"), "dark") })
        {
            var appearance = new ToggleMenuFlyoutItem { Text = design.L("外观：", "Appearance: ") + label, IsChecked = settings.Appearance == value, IsEnabled = !changingAppearance };
            appearance.Click += async (_, _) => await ChangeAppearanceAsync(value); menu.Items.Add(appearance);
        }
        Register(menu); switcher.Flyout = menu; heading.Children.Add(switcher);
        if (collapseButton != null) heading.Children.Add(collapseButton);
        top.Children.Add(heading);
        var navigation = Stack(rail ? 4 : 2); top.Children.Add(navigation);
        navigation.Children.Add(SidebarAction(design.T("sidebar.new_session"), "plus", () => _ = CreateSessionAsync(), "Ctrl+N"));
        navigation.Children.Add(SidebarAction(design.T("sidebar.search"), "search", OpenSearch, "Ctrl+K"));
        var research = SidebarAction(design.T("sidebar.more"), "more", () => { });
        if (research.Content is Grid researchContent)
        {
            var caret = design.Icon("chevron-right", 14, design.ColorText("text-faint")); Grid.SetColumn(caret, 2); researchContent.Children.Add(caret);
        }
        var researchMenu = new MenuFlyout { Placement = FlyoutPlacementMode.RightEdgeAlignedTop };
        var researchMenuStyle = new Style(typeof(MenuFlyoutPresenter));
        researchMenuStyle.Setters.Add(new Setter(FrameworkElement.MinWidthProperty, 184d));
        researchMenuStyle.Setters.Add(new Setter(Control.PaddingProperty, new Thickness(6)));
        researchMenuStyle.Setters.Add(new Setter(Control.BackgroundProperty, design.Brush("bg-elev")));
        researchMenuStyle.Setters.Add(new Setter(Control.BorderBrushProperty, design.Brush("border-strong")));
        researchMenuStyle.Setters.Add(new Setter(Control.BorderThicknessProperty, new Thickness(1)));
        researchMenuStyle.Setters.Add(new Setter(Control.CornerRadiusProperty, new CornerRadius(8)));
        researchMenu.MenuFlyoutPresenterStyle = researchMenuStyle;
        var entries = new List<(string Id, string Icon, int Tier, Button Inline, MenuFlyoutItem Folded)>();
        void AddEntry(string id, string key, string icon, int tier, Action? action)
        {
            var label = design.T(key); var inline = SidebarAction(label, icon, action); navigation.Children.Add(inline);
            var folded = new MenuFlyoutItem { Text = label, Icon = new ImageIcon { Source = design.Icon(icon, 16).Source }, IsEnabled = action != null,
                MinHeight = 34, Padding = new Thickness(10, 8, 10, 8), Margin = new Thickness(0, 1, 0, 1), CornerRadius = new CornerRadius(8) };
            design.BindTypography(folded, size: 13.5);
            folded.Resources["MenuFlyoutItemBackgroundPointerOver"] = design.Brush("bg-sunken");
            folded.Click += (_, _) => { if (workspaceSheet?.KeepsProjectSidebar == true && !workspaceSheet.IsModal) ClearSheets(); action?.Invoke(); };
            researchMenu.Items.Add(folded); entries.Add((id, icon, tier, inline, folded));
        }
        AddEntry("folder", "sidebar.new_folder", "folder-plus", 2, sessionGroups == null ? null : () => _ = EditGroup());
        AddEntry("files", "sidebar.files", "doc", 2, ShowFiles);
        AddEntry("journey", "sidebar.graph", "research-trail", 1, () => _ = OpenNativeAction("journey"));
        AddEntry("memos", "sidebar.memos", "notebook", 1, () => ShowPanelTab("memos"));
        AddEntry("publication", "sidebar.publication", "book", 1, () => _ = OpenNativeAction("publication"));
        AddEntry("library", "sidebar.library", "star", 1, () => _ = OpenNativeAction("library"));
        Register(researchMenu); research.Flyout = researchMenu; navigation.Children.Add(research);
        var previousTier = -1;
        void FitNavigation()
        {
            var height = root.ActualHeight;
            var tier = PreviewLayout.SidebarEntryFolded(2, height) ? 2 : PreviewLayout.SidebarEntryFolded(1, height) ? 1 : 0;
            if (tier != previousTier) researchMenu.Hide();
            var returnInlineFocus = research.FocusState != FocusState.Unfocused && tier == 0;
            var returnMoreFocus = entries.Any(entry => entry.Inline.FocusState != FocusState.Unfocused && PreviewLayout.SidebarEntryFolded(entry.Tier, height));
            var foldedActive = false;
            foreach (var entry in entries)
            {
                var folded = PreviewLayout.SidebarEntryFolded(entry.Tier, height);
                entry.Inline.Visibility = folded ? Visibility.Collapsed : Visibility.Visible;
                entry.Folded.Visibility = folded ? Visibility.Visible : Visibility.Collapsed;
                var active = entry.Id == "journey" && workspaceSheet is NativeJourneyPage { KeepsProjectSidebar: true }
                    || entry.Id == "publication" && projectPage is NativePublicationPage
                    || entry.Id == "memos" && panelVisible && settings.PanelTab == "memos";
                foldedActive |= active && folded;
                entry.Inline.Background = active ? design.Brush("clay-soft") : new SolidColorBrush(Microsoft.UI.Colors.Transparent);
                if (entry.Inline.Content is Grid content)
                {
                    if (content.Children[1] is TextBlock caption) caption.Foreground = design.Brush(active ? "clay-strong" : "text-muted");
                    if (content.Children[0] is Image icon) icon.Source = design.Icon(entry.Icon, 16, design.ColorText(active ? "clay-strong" : "text-muted")).Source;
                }
                else if (entry.Inline.Content is Image glyph)
                    glyph.Source = design.Icon(entry.Icon, 17, design.ColorText(active ? "clay-strong" : "text-muted")).Source;
                entry.Folded.Foreground = design.Brush(active ? "clay-strong" : "text-muted");
                entry.Folded.Background = active ? design.Brush("clay-soft") : new SolidColorBrush(Microsoft.UI.Colors.Transparent);
                if (entry.Folded.Icon is ImageIcon foldedIcon) foldedIcon.Source = design.Icon(entry.Icon, 16, design.ColorText(active ? "clay-strong" : "text-muted")).Source;
            }
            research.Visibility = tier > 0 ? Visibility.Visible : Visibility.Collapsed;
            research.Background = foldedActive ? design.Brush("clay-soft") : new SolidColorBrush(Microsoft.UI.Colors.Transparent);
            if (research.Content is Grid moreContent)
            {
                if (moreContent.Children[1] is TextBlock moreLabel) moreLabel.Foreground = design.Brush(foldedActive ? "clay-strong" : "text-muted");
                if (moreContent.Children[0] is Image moreIcon) moreIcon.Source = design.Icon("more", 16, design.ColorText(foldedActive ? "clay-strong" : "text-muted")).Source;
            }
            else if (research.Content is Image glyph)
                glyph.Source = design.Icon("more", 17, design.ColorText(foldedActive ? "clay-strong" : "text-muted")).Source;
            if (returnMoreFocus) research.Focus(FocusState.Keyboard);
            else if (returnInlineFocus) entries.First(entry => entry.Tier == 1).Inline.Focus(FocusState.Keyboard);
            previousTier = tier;
        }
        refreshSidebarNavigation = FitNavigation;
        sidebar.Loaded += (_, _) => FitNavigation(); sidebar.SizeChanged += (_, _) => FitNavigation(); sidebar.Unloaded += (_, _) => researchMenu.Hide();
        FitNavigation();
        sidebar.Children.Add(top);
        var sessionSection = SessionList();
        Grid.SetRow(sessionSection, 1); sidebar.Children.Add(sessionSection);
        var bottom = Stack(2);
        bottom.Children.Add(new Border { BorderBrush = design.Brush("border"), BorderThickness = new Thickness(0, 1, 0, 0), Margin = new Thickness(0, 0, 0, 8) });
        inventoryLabel = null;
        if (!layout.CompactWorkspace)
        {
            inventoryLabel = Text("", 11, "text-faint");
            inventoryLabel.Margin = new Thickness(12, 0, 8, 6);
            RefreshInventoryLabel(); bottom.Children.Add(inventoryLabel);
        }
        bottom.Children.Add(SidebarAction(design.T("sidebar.capabilities"), "grid", () => _ = OpenNativeAction("capabilities")));
        bottom.Children.Add(SidebarAction(design.T("issue_report.sidebar"), "chat", model.ActiveSessionId == null ? null : () => _ = conversation?.PrepareIssueReportAsync()));
        bottom.Children.Add(SidebarAction(design.T("sidebar.settings"), "gear", OpenSettings));
        Grid.SetRow(bottom, 2); sidebar.Children.Add(bottom); return sidebar;
    }

    private Button SidebarAction(string label, string icon, Action? action, string? shortcut = null)
    {
        var content = new Grid { ColumnSpacing = 10 };
        content.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
        content.ColumnDefinitions.Add(new() { Width = new GridLength(1, GridUnitType.Star) });
        content.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
        content.Children.Add(design.Icon(icon, 16));
        var title = SingleLine(label, 13.5, "text-muted"); Grid.SetColumn(title, 1); content.Children.Add(title);
        if (shortcut != null)
        {
            var key = Text(shortcut, 11, "text-faint"); key.VerticalAlignment = VerticalAlignment.Center;
            Grid.SetColumn(key, 2); content.Children.Add(key);
        }
        var button = ContentButton(content, () => { if (icon != "more" && workspaceSheet?.KeepsProjectSidebar == true && !workspaceSheet.IsModal) ClearSheets(); action?.Invoke(); }, "sidebar-" + icon, label); button.IsEnabled = action != null;
        design.QuietButton(button); button.Padding = new Thickness(10, 8, 10, 8);
        button.HorizontalAlignment = HorizontalAlignment.Stretch; button.HorizontalContentAlignment = HorizontalAlignment.Stretch;
        if (layout.CompactWorkspace)
        {
            button.Content = design.Icon(icon, 17, design.ColorText("text-muted")); button.MinWidth = 0; button.MinHeight = 36;
            button.HorizontalContentAlignment = HorizontalAlignment.Center; button.Padding = new Thickness(8);
            ToolTipService.SetToolTip(button, shortcut == null ? label : label + "  " + shortcut);
        }
        return button;
    }

    private FrameworkElement Footer(bool workspace = false)
    {
        var footer = Stack(6);
        var actions = Row(8);
        var refresh = ActionButton(design.L("刷新", "Refresh"), "refresh", () => { localError = null; _ = model.RefreshAsync(); }, quiet: true); refresh.IsEnabled = !model.Loading;
        actions.Children.Add(refresh);
        actions.Children.Add(ActionButton(design.L("选择数据库", "Choose database"), "database", ChooseDatabase, !workspace, quiet: true));
        var appearance = ActionButton(changingAppearance ? "正在保存外观…" : "外观", null, () => { }, quiet: true);
        appearance.IsEnabled = !changingAppearance;
        var menu = new MenuFlyout();
        foreach (var (label, value) in new[] { (design.L("跟随系统", "System"), "system"), (design.L("浅色", "Light"), "light"), (design.L("深色", "Dark"), "dark") })
        {
            var item = new ToggleMenuFlyoutItem { Text = label, IsChecked = settings.Appearance == value };
            item.Click += async (_, _) => await ChangeAppearanceAsync(value);
            menu.Items.Add(item);
        }
        Register(menu); appearance.Flyout = menu; actions.Children.Add(appearance);
        if (!workspace && model.LastLoaded is { } time) actions.Children.Add(Text(design.L($"更新于 {time:HH:mm:ss}", $"Updated {time:HH:mm:ss}"), 11, "text-faint"));
        footer.Children.Add(actions); return footer;
    }

    private void OpenSearch() => OpenSearch(false);
    private void OpenSearch(bool commandsOnly)
    {
        if (searchOverlay != null || nativePickerOpen || !(commandsOnly && SetupCommandLayer) && (settingsPage != null || workspaceSheet is not (null or NativeProjectChatSearch))) return;
        searchPreviousFocus = FocusManager.GetFocusedElement(root.XamlRoot) as Control;
        var database = model.DatabasePath;
        var project = model.ActiveProjectId;
        var session = model.ActiveSessionId;
        var paletteDesign = settingsPage?.OverlayDesign ?? design;
        WorkspaceSearchModel search = null!;
        search = new WorkspaceSearchModel(async () =>
        {
            var host = await ConnectHostAsync();
            if (host == null || windowClosed || database != model.DatabasePath) return null;
            if (!commandsOnly) await search.LoadCommandProjectAsync(host, model.Projects.Select(p => (p.Id, p.WorkspaceDirectory)).ToArray());
            return windowClosed || database != model.DatabasePath ? null : new NativeSearchClient(host);
        }, project, paletteDesign.Localization);
        searchOverlay = new ProjectSearchOverlay(search, paletteDesign, CloseSearch,
            (result, attach, newWindow) => { if (!newWindow) LeaveUnderlyingChatHistory(); _ = OpenSearchResultAsync(result, attach, newWindow, database, project, session); },
            conversation?.CanAttach == true && conversation.Snapshot?.ComposerReferences == true, Register,
            action =>
            {
                if (action == "search-project-chats" && workspaceSheet is NativeProjectChatSearch) return;
                if (workspaceSheet is NativeQuickSetupPage) CloseSheet();
                if (settingsPage is { } prefs)
                {
                    if (action == "privacy-mode") { _ = OpenPrivacyAsync(); return; }
                    if (action == "setup") { _ = OpenQuickSetupAsync(); return; }
                    if (action is "settings" or "skills") { prefs.SelectSection(action == "settings" ? "models" : "skills"); return; }
                    prefs.RequestLeave(() => { CloseSettings(); if (workspaceSheet is NativeAssistantPage) CloseSheet(); RunWindowCommand(action); });
                    return;
                }
                LeaveUnderlyingChatHistory();
                if (action is "project-settings" or "skills" && search.CommandProjectId is { } commandProject)
                {
                    OpenSettingsSection(action == "project-settings" ? "project" : "skills", commandProject); return;
                }
                RunWindowCommand(action);
            }, project != null, session != null, commandsOnly, hostVersion,
            id => model.Projects.FirstOrDefault(p => p.Id == id)?.Description);
        if (workspaceSheet is UserControl parent) parent.IsHitTestVisible = false;
        modalLayer.Children.Add(searchOverlay); UpdateSheetPresentation();
    }

    private void LeaveUnderlyingChatHistory()
    {
        if (workspaceSheet is not NativeProjectChatSearch) return;
        CloseSheet();
        if (workspaceSheet is NativeShelvedSessionsPage) CloseSheet();
    }

    private void CloseSearch()
    {
        if (searchOverlay == null) return;
        searchOverlay.Dispose(); modalLayer.Children.Remove(searchOverlay); searchOverlay = null;
        if (workspaceSheet is UserControl parent) parent.IsHitTestVisible = true;
        UpdateSheetPresentation();
        searchPreviousFocus?.Focus(FocusState.Programmatic);
    }

    private async void ChooseDatabase()
    {
        if (changingAppearance || nativePickerOpen || searchOverlay != null || settingsPage != null || workspaceSheet != null) return;
        nativePickerOpen = true;
        try
        {
            var picker = new FileOpenPicker();
            WinRT.Interop.InitializeWithWindow.Initialize(picker, WinRT.Interop.WindowNative.GetWindowHandle(this));
            picker.FileTypeFilter.Add(".sqlite"); picker.FileTypeFilter.Add(".db"); picker.FileTypeFilter.Add("*");
            if (await picker.PickSingleFileAsync() is { } file)
            {
                appearanceGeneration++;
                settings.DatabasePath = file.Path; SaveSettings(); localError = null;
                DisposeWorkspace(); conversation = null;
                workspaceHost?.Dispose(); workspaceHost = null;
                homeSupportedHost = null; homeRetryAt = default;
                homeSync = new(); syncBackendConfigured = false; homeSyncNotice = restoreSyncFocus = null;
                storageBrowser = new();
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
        projectInventory.Select(model.DatabasePath, model.ActiveProjectId);
        if (model.ActiveProjectId == null) inventoryLabel = null;
        if (projectMemos != null && projectMemos.ProjectId != model.ActiveProjectId) { projectMemos.Dispose(); projectMemos = null; }
        SyncProjectActions();
        var key = model.ActiveProjectId is { } project ? project + ":" + model.ActiveSessionId : null;
        if (key == workspaceKey) return;
        runNavigation++;
        workspaceKey = key;
        savedChatSequence = null; scrollToChatHit = false;
        ClearCenterArtifacts();
        ClearSheets();
        panelPage?.Dispose(); panelPage = null;
        terminalPage?.Dispose(); terminalPage = null;
        sideChat = null;
        inbox?.Reset();
        conversation?.Reset();
        conversationReady = model.ActiveProjectId != null ? EnsureConversationAsync() : Task.CompletedTask;
        if (panelVisible || terminalVisible) _ = EnsurePanelAndTerminalAsync();
    }

    private async Task EnsureConversationAsync()
    {
        var navigation = ++conversationNavigation;
        var selectedProject = model.ActiveProjectId; var selectedSession = model.ActiveSessionId;
        var host = await ConnectHostAsync();
        if (host == null || windowClosed || navigation != conversationNavigation || selectedProject != model.ActiveProjectId || selectedSession != model.ActiveSessionId) return;
        _ = ReadInventoryAsync(host);
        conversation ??= new WorkspaceConversationModel(new NativeConversationClient(host), host);
        conversation.SnapshotAccepted -= UpdateSessionActivity;
        conversation.SnapshotAccepted += UpdateSessionActivity;
        conversation.ApplyInputPreferences(settings.InputPreferences ?? new());
        conversationPage ??= new NativeConversationPage(conversation, design, QuoteSelection, CreateSessionAsync, () => PickFile("*"),
            RouteSlashCommand, () => ShowPanelTab("hosts"), id => _ = OpenRunAsync(id), CreateAcpSessionAsync, OpenHistoryBranchAsync, id => OpenRunAsync(id, true),
            () => !windowClosed && !nativePickerOpen && overlays.Count == 0 && searchOverlay == null && workspaceSheet == null
                && settingsPage == null && !panelVisible, EnsureComposerSessionAsync, PickAttachments,
            action => { if (action == "files") ShowPanelTab("files"); else if (action == "share") _ = OpenSheet("share"); else if (action == "skills") OpenSettingsSection("skills"); },
            () => !windowClosed && !model.Loading && model.ActiveProjectId is { } scopeProject
                ? new NativeSessionReferenceScope(scopeProject, model.Projects.FirstOrDefault(p => p.Id == scopeProject)?.Name ?? "", model.Sessions.ToArray()) : null,
            OpenTranscriptBranch, OpenBranchResult, summary => _ = ReviewExplorationAsync(summary), target => _ = StartExplorationAsync(target), OpenExcerptAsync,
            SendQueuedSideChat, CanSendQueuedSideChat, () => OpenSettingsSection("hooks"), () => OpenSettingsSection("environments"), () => OpenComputeHostDialog(model.ActiveProjectId), () => OpenSettingsSection("models"),
            () => settings.ModelSwitchWarningDisabled, () => { settings.ModelSwitchWarningDisabled = true; SaveSettings(); }, StartHandoffAsync, StartContextRecoveryAsync,
            () => !windowClosed && !nativePickerOpen && overlays.Count == 0 && searchOverlay == null && workspaceSheet == null && settingsPage == null,
            output => _ = OpenGeneratedAsync(output), () => model.Projects.FirstOrDefault(project => project.Id == model.ActiveProjectId)?.WorkspaceDirectory ?? "");
        if (model.ActiveProjectId is { } project && model.ActiveSessionId is { } session)
        {
            sideChat = new WorkspaceSideChatModel(new NativeSideChatClient(host), project, session);
            var lastActivity = model.Sessions.FirstOrDefault(row => row.ProjectId == project && row.Id == session)?.Timestamp ?? 0;
            await conversation.OpenAsync(project, session, lastActivity: lastActivity);
        }
        else if (model.ActiveProjectId is { } draftProject) await conversation.OpenProjectDraftAsync(draftProject);
        else conversation.Reset();
        if (!windowClosed) Render();
    }

    private async Task<bool> EnsureComposerSessionAsync()
    {
        var owner = conversation; var project = model.ActiveProjectId; var selected = model.ActiveSessionId;
        if (owner == null || project == null || owner.CurrentProjectId != project) return false;
        if (selected != null) return owner.Snapshot is { ReadOnly: false } ready && ready.SessionId == selected;
        try
        {
            var id = owner.CurrentSessionId ?? await owner.EnsureDraftSessionAsync();
            var prepared = id != null;
            // Adopt the confirmed frame for recovery without continuing the first send.
            if (!prepared && owner.DraftFastFailureDetail != null) id = owner.CurrentSessionId;
            if (id == null || windowClosed || conversation != owner || model.ActiveProjectId != project
                || model.ActiveSessionId != null || owner.CurrentProjectId != project || owner.CurrentSessionId != id) return false;
            if (owner.Snapshot == null) await owner.RefreshAsync();
            if (windowClosed || conversation != owner || model.ActiveProjectId != project || model.ActiveSessionId != null
                || owner.Snapshot is not { ReadOnly: false } snapshot || snapshot.ProjectId != project || snapshot.SessionId != id) return false;
            // Adopt the already-open model. Resetting it here would discard the first draft.
            workspaceKey = project + ":" + id;
            sideChat = new WorkspaceSideChatModel(new NativeSideChatClient(workspaceHost!), project, id);
            await model.OpenCreatedSessionAsync(project, id);
            return prepared && !windowClosed && conversation == owner && model.ActiveProjectId == project && model.ActiveSessionId == id
                && owner.CurrentProjectId == project && owner.CurrentSessionId == id;
        }
        catch (Exception ex)
        {
            if (!windowClosed && model.ActiveProjectId == project && conversation == owner)
            { localError = ex.Message; Render(); }
            return false;
        }
    }

    private void QuoteSelection(string text)
    {
        if (sideChat is null) return;
        runNavigation++;
        sideChat.Quotes.Add(new NativeSideChatQuote(text, "会话摘录"));
        panelVisible = true; settings.PanelVisible = true; settings.PanelTab = "sidechat"; SaveSettings();
        if (panelPage == null) _ = EnsurePanelAndTerminalAsync();
        else panelPage.Refresh();
    }

    private bool CanSendQueuedSideChat() => !windowClosed && sideChat is { Busy: false, ChangingModel: false, ModelSelectionUncertain: false }
        && string.IsNullOrEmpty(sideChat.Draft) && sideChat.Quotes.Count == 0;
    private bool SendQueuedSideChat(NativeQueueTarget target, string text)
    {
        if (!CanSendQueuedSideChat() || model.ActiveProjectId != target.ProjectId || model.ActiveSessionId != target.SessionId
            || conversation?.CurrentProjectId != target.ProjectId || conversation.CurrentSessionId != target.SessionId || string.IsNullOrWhiteSpace(text)) return false;
        var owner = sideChat!; owner.Draft = text;
        ShowPanelTab("sidechat");
        _ = SendAsync(); return true;
        async Task SendAsync()
        {
            await owner.SendAsync();
            if (!windowClosed && sideChat == owner) panelPage?.Refresh();
        }
    }

    private Task CreateSessionAsync() => CreateSessionWithAgentAsync(null);
    private async Task StartEnvironmentSetupAsync(string project)
    {
        var owner = conversation; var source = model.ActiveSessionId; var database = model.DatabasePath;
        var navigation = conversationNavigation;
        bool Current() => !windowClosed && owner == conversation && database == model.DatabasePath && project == model.ActiveProjectId;
        if (owner == null || !owner.CanStartPromptSession(project) || !Current()) return;
        var prompt = design.T("caps.env_setup_prompt");
        CloseSheet();
        try
        {
            var id = await owner.CreatePromptSessionAsync(project, prompt);
            if (!Current() || navigation != conversationNavigation || model.ActiveSessionId != source) return;
            await model.RefreshAsync();
            if (!Current() || navigation != conversationNavigation || model.ActiveSessionId != source) return;
            await model.OpenCreatedSessionAsync(project, id);
            if (!Current() || model.ActiveSessionId != id) return;
            await conversationReady;
            if (!Current() || model.ActiveSessionId != id || owner.CurrentSessionId != id || owner.Draft != prompt) return;
            await owner.SendAsync(); conversationPage?.Refresh();
        }
        catch (Exception ex) { if (Current()) { localError = ex.Message; Render(); } }
    }
    private async Task OpenExcerptAsync(string project, string source, string destination)
    {
        var navigation = conversationNavigation; var owner = conversation;
        bool Current() => !windowClosed && conversation == owner && navigation == conversationNavigation
            && model.ActiveProjectId == project && model.ActiveSessionId == source;
        if (!Current()) return;
        await model.RefreshAsync();
        if (!Current()) return;
        await model.OpenProjectAsync(project, destination);
        if (!windowClosed && model.ActiveProjectId == project && model.ActiveSessionId == destination)
            conversationPage?.FocusComposer();
    }
    private async Task OpenHistoryBranchAsync(string project, string source, string branch)
    {
        if (windowClosed || model.ActiveProjectId != project || model.ActiveSessionId != source) return;
        await model.RefreshAsync();
        if (windowClosed || model.ActiveProjectId != project || model.ActiveSessionId != source) return;
        await model.OpenProjectAsync(project, branch);
    }
    private async Task CreateAcpSessionAsync(string agentId)
    {
        var owner = conversation; var project = model.ActiveProjectId; var source = model.ActiveSessionId;
        if (windowClosed || owner == null || project == null || owner.CurrentProjectId != project || owner.CurrentSessionId != source) return;
        var id = await owner.CreateAcpDraftAsync(agentId);
        if (id == null || windowClosed || conversation != owner || model.ActiveProjectId != project || model.ActiveSessionId != source
            || owner.CurrentProjectId != project || owner.CurrentSessionId != id) return;
        workspaceKey = project + ":" + id;
        sideChat = new WorkspaceSideChatModel(new NativeSideChatClient(workspaceHost!), project, id);
        await model.OpenCreatedSessionAsync(project, id);
        if (!windowClosed && conversation == owner && model.ActiveProjectId == project && model.ActiveSessionId == id)
            conversationPage?.FocusComposer();
    }
    private async Task CreateSessionWithAgentAsync(string? agentId)
    {
        if (model.ActiveProjectId is not { } project) return;
        var host = await ConnectHostAsync();
        if (host == null) return;
        conversation ??= new WorkspaceConversationModel(new NativeConversationClient(host), host);
        if (windowClosed || model.ActiveProjectId != project) return;
        var id = await conversation.CreateAsync(project, agentId: agentId);
        if (id is null) { conversationPage?.Refresh(); return; }
        if (windowClosed || model.ActiveProjectId != project) return;
        await model.RefreshAsync();
        await model.OpenCreatedSessionAsync(project, id);
    }

    private async Task StartHandoffAsync(NativeHandoffLaunch launch)
    {
        var owner = conversation; var database = model.DatabasePath;
        bool SameSource() => !windowClosed && conversation == owner && database == model.DatabasePath
            && model.ActiveProjectId == launch.ProjectId && model.ActiveSessionId == launch.SessionId
            && owner?.Snapshot?.Epoch == launch.Epoch && !owner.ShowingHistory;
        if (owner == null || !SameSource()) throw new InvalidOperationException("The source conversation changed.");
        var id = await owner.PrepareHandoffSessionAsync(launch);
        if (!SameSource()) return; // The known destination keeps its draft; never send after navigation.
        await model.RefreshAsync();
        if (!SameSource()) return;
        await model.OpenCreatedSessionAsync(launch.ProjectId, id);
        if (windowClosed || database != model.DatabasePath || model.ActiveProjectId != launch.ProjectId || model.ActiveSessionId != id) return;
        await EnsureConversationAsync();
        if (windowClosed || database != model.DatabasePath || conversation != owner || model.ActiveProjectId != launch.ProjectId
            || model.ActiveSessionId != id || owner.CurrentSessionId != id || owner.Draft != launch.Note.Trim()) return;
        await owner.SendAsync();
        conversationPage?.Refresh();
    }

    private async Task StartContextRecoveryAsync(NativeContextRecoveryLaunch launch)
    {
        var owner = conversation; var database = model.DatabasePath; var source = launch.Source;
        bool SameSource() => !windowClosed && conversation == owner && database == model.DatabasePath
            && model.ActiveProjectId == source.ProjectId && model.ActiveSessionId == source.SessionId
            && owner?.RecoverySourceMatches(source) == true && !owner.ShowingHistory;
        if (owner == null || !SameSource()) throw new InvalidOperationException("The source conversation changed.");
        var id = await owner.PrepareContextRecoverySessionAsync(launch);
        if (!SameSource()) return;
        await model.RefreshAsync();
        if (!SameSource()) return;
        await model.OpenCreatedSessionAsync(source.ProjectId, id);
        if (windowClosed || database != model.DatabasePath || model.ActiveProjectId != source.ProjectId || model.ActiveSessionId != id) return;
        await EnsureConversationAsync();
        if (windowClosed || database != model.DatabasePath || conversation != owner || model.ActiveProjectId != source.ProjectId
            || model.ActiveSessionId != id || owner.CurrentSessionId != id || owner.Draft != launch.Prompt
            || owner.References.Length != 1 || owner.References[0].Reference != new NativeComposerReference("session", source.SessionId)) return;
        await owner.SendAsync();
        conversationPage?.Refresh();
    }

    private async Task EnsurePanelAndTerminalAsync()
    {
        if (model.ActiveProjectId is not { } project) return;
        var session = model.ActiveSessionId;
        Render();
        var host = await ConnectHostAsync();
        if (host == null || windowClosed || model.ActiveProjectId != project || model.ActiveSessionId != session) return;
        if (conversation == null) await EnsureConversationAsync();
        if (windowClosed || model.ActiveProjectId != project || model.ActiveSessionId != session) return;
        var panelClient = new NativePanelClient(host);
        if (panelVisible && panelPage == null)
        {
            var tabs = session == null ? new NativePanelTabs(settings.PanelTabs, settings.PanelTab, ["files", "memos", "runs", "hosts"])
                : new NativePanelTabs(settings.PanelTabs, settings.PanelTab, NativePanelTabs.All);
            if (session == null) tabs.Show(settings.PanelTab is NativePanelTabs.Home or "memos" or "runs" or "hosts" ? settings.PanelTab : "files");
            projectMemos ??= new WorkspaceMemosModel(new NativeMemosClient(host), project);
            if (session != null) sideChat ??= new WorkspaceSideChatModel(new NativeSideChatClient(host), project, session);
            panelPage = new NativeWorkspacePanel(new WorkspacePanelModel(panelClient, project, session, tabs,
                new NativeHighlightClient(host), new NativeNotebookClient(host), new NativeAgentPanelClient(host),
                new NativeContextActivityClient(host), new NativeFilesClient(host)),
                conversation,
                design, TogglePanel, sideChat, session == null ? null : context => _ = OpenTerminalContextAsync(context),
                (selected, saved) => { runNavigation++; settings.PanelTab = selected; if (session != null) settings.PanelTabs = saved; SaveSettings(); },
                session == null ? null : new WorkspaceRunReviewModel(new NativeRunReviewClient(host), project, session), () => settings.InputPreferences ?? new(),
                (path, selection) =>
                {
                    if (session == null || windowClosed || !panelVisible || model.ActiveProjectId != project || model.ActiveSessionId != session
                        || conversation?.AddDocumentQuote(project, session, path, selection) != true) return false;
                    if (selection.Jump)
                        DispatcherQueue.TryEnqueue(() =>
                        {
                            if (windowClosed || model.ActiveProjectId != project || model.ActiveSessionId != session) return;
                            if (panelVisible) TogglePanel();
                            conversationPage?.FocusComposer();
                        });
                    return true;
                }, HandleWindowEscape, PickAttachments, PickFileDownload,
                (reader, id) =>
                {
                    if (windowClosed || model.ActiveProjectId != project || model.ActiveSessionId != session) { reader.Close(); return; }
                    OpenCenterArtifact(reader, id);
                }, text => InsertArtifactDraft(project, session, text), projectMemos, text =>
                {
                    if (windowClosed || model.ActiveProjectId != project || model.ActiveSessionId != session || conversation?.InsertMemoBody(text) != true) return false;
                    DispatcherQueue.TryEnqueue(() => { if (!windowClosed && model.ActiveProjectId == project && model.ActiveSessionId == session) conversationPage?.FocusComposer(); });
                    return true;
                }, () => model.Projects.FirstOrDefault(item => item.Id == project)?.WorkspaceDirectory ?? "",
                () => OpenSettingsSection("environments"),
                (contextId, trigger, configure) => conversationPage?.ShowPanelRuntimesAsync(contextId, trigger, configure) ?? Task.CompletedTask,
                run => {
                    if (windowClosed || model.ActiveProjectId != project || model.ActiveSessionId != session || workspaceHost != host) return;
                    ShowPublication(host, project, new(project, run.Id, string.IsNullOrWhiteSpace(run.Title) ? run.Id : run.Title));
                }, () => OpenSettingsSection("workflows"));
        }
        if (session != null && terminalVisible && terminalPage == null)
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
        runNavigation++;
        panelVisible = !panelVisible;
        if (!panelVisible) panelPage?.CloseRunReview();
        settings.PanelVisible = panelVisible; SaveSettings();
        if (panelVisible && panelPage == null) _ = EnsurePanelAndTerminalAsync();
        else { if (panelVisible) panelPage?.ReopenTabs(); Render(); }
    }

    private void ToggleTerminal()
    {
        terminalVisible = !terminalVisible;
        if (terminalVisible && terminalPage == null) _ = EnsurePanelAndTerminalAsync();
        else Render();
    }

    private async Task OpenTerminalContextAsync(string context)
    {
        var project = model.ActiveProjectId; var session = model.ActiveSessionId;
        terminalVisible = true;
        if (terminalPage == null) await EnsurePanelAndTerminalAsync();
        if (windowClosed || model.ActiveProjectId != project || model.ActiveSessionId != session || terminalPage == null) return;
        await terminalPage.OpenContextAsync(context);
        if (!windowClosed) Render();
    }

    private async Task OpenSheet(string kind)
    {
        if (workspaceSheet != null || settingsPage != null) return;
        var project = model.ActiveProjectId;
        var session = model.ActiveSessionId;
        if (kind != "inbox" && (project is null || session is null)) return;
        var loading = new NativeConnectionPage(design, kind switch { "outline" => design.L("会话大纲", "Conversation outline"), "share" => design.T("share.topbar"), "trajectory" => design.T("trajectory.topbar"), "archive" => design.L("研究归档", "Archive research"), _ => design.L("待查看", "Need you") }, CloseSheet,
            () => { CloseSheet(); _ = OpenSheet(kind); });
        MountSheet(loading);
        var host = await ConnectHostAsync();
        if (windowClosed || !ReferenceEquals(workspaceSheet, loading) || project != model.ActiveProjectId || session != model.ActiveSessionId) return;
        if (host == null) { loading.Failed(localError ?? "连接失败，请重试。"); return; }
        IWorkspaceSheet page = kind switch
        {
            "outline" => new NativeOutlinePage(new WorkspaceOutlineModel(new NativeConversationClient(host), project!, session!), design, async (entry, token) =>
            {
                if (windowClosed || model.ActiveProjectId != project || model.ActiveSessionId != session || token.IsCancellationRequested
                    || conversation is not { } targetConversation)
                    throw new OperationCanceledException();
                if (!await targetConversation.OpenQuestionAsync(entry, token))
                {
                    token.ThrowIfCancellationRequested();
                    throw new InvalidOperationException(targetConversation.OperationError ?? "问题位置已变化，请刷新大纲后重试。");
                }
            }, CloseSheet),
            "share" => new NativeSharePage(new WorkspaceShareModel(new NativeShareClient(host), project!, session!), design,
                SaveSharedFileAsync, CloseSheet),
            "trajectory" => new NativeTrajectoryPage(new WorkspaceTrajectoryModel(new NativeConversationClient(host), project!, session!), design,
                (name, html) => NativeWorkspaceFiles.SaveHtmlAsync(this, name, html), CloseSheet),
            "archive" => new NativeArchivePage(new WorkspaceArchiveModel(new NativeArchiveClient(host), project!, session!), design, async id =>
            {
                CloseSheet();
                if (model.ActiveProjectId is { } current) await model.OpenProjectAsync(current, id);
            }, CloseSheet),
            _ => CreateInboxPage(host, project ?? "")
        };
        CloseSheet(); MountSheet(page);
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
        if (workspaceSheet is UserControl control) SheetHost(workspaceSheet).Children.Remove(control);
        workspaceSheet.Dispose(); workspaceSheet = null;
        if (parentSheets.TryPop(out var parent))
        {
            workspaceSheet = parent.Sheet;
            if (parent.Sheet is UserControl restored)
            {
                restored.IsHitTestVisible = true;
                var host = SheetHost(parent.Sheet);
                if (!host.Children.Contains(restored)) host.Children.Add(restored);
            }
            parent.Focus?.Focus(FocusState.Programmatic);
        }
        UpdateSheetPresentation();
        if (workspaceSheet == null && modalPreviousFocus != null)
        {
            if (!modalPreviousFocus.Focus(FocusState.Programmatic) && pageContent != null)
                (FocusManager.FindFirstFocusableElement(pageContent) as Control)?.Focus(FocusState.Programmatic);
            modalPreviousFocus = null;
        }
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
            localError = null;
            await model.RefreshAfterHostReadyAsync(database);
            if (windowClosed || database != model.DatabasePath || workspaceHost != connected) return null;
            await ReadAppearanceAsync(connected);
            UpdateHomePolling();
            _ = ReadWindowVersionAsync(connected);
            return connected;
        }
        catch (Exception ex)
        {
            if (windowClosed || database != model.DatabasePath) return null;
            localError = NativeBrowserPresentation.ConnectionError(ex);
            model.ApplyHomeActivity(null);
            Render();
            return null;
        }
    }

    private void DisposeWorkspace()
    {
        projectInventory.Select("", null, refresh: true); inventoryLabel = null;
        projectMemos?.Dispose(); projectMemos = null;
        runNavigation++;
        ClearCenterArtifacts();
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
        => OpenSettingsSection("general");

    private void OpenSettingsSection(string initialSection, string? projectId = null, bool preserveAssistant = false)
    {
        if (projectImport?.IsModal == true) return;
        if (initialSection == "project") { _ = OpenProjectSettingsAsync(projectId ?? model.ActiveProjectId); return; }
        if (settingsPage != null || changingAppearance) return;
        // Settings hides the retained workbench without unloading it. Popups have
        // their own visual root and must be dismissed before hiding that page.
        conversationPage?.DismissRuntimeSurfaces();
        appearanceGeneration++;
        runNavigation++;
        settingsAssistantProject = model.ActiveProjectId; settingsAssistantSession = model.ActiveSessionId;
        if (!preserveAssistant || workspaceSheet is not NativeAssistantPage) CloseSheet();
        else if (workspaceSheet is Control retainedAssistant) retainedAssistant.IsEnabled = false;
        IReadOnlyList<ProjectSummary> settingsProjects = model.Projects;
        if (preserveAssistant && projectId == NativeAssistantClient.ProjectId)
            settingsProjects = model.Projects.Append(new ProjectSummary(NativeAssistantClient.ProjectId, design.T("assistant.title"),
                "", "", false, 1, 0, 0, 0, 0, false, null)).ToArray();
        settingsPage = new NativeSettingsPage(model.DatabasePath, projectId ?? model.ActiveProjectId, ApplyAppearancePreferences,
            CloseSettings, initialSection, settingsProjects, folder => folder ? PickDirectory() : PickFile("*"), design.Typography,
            async () => { await appearancePersistence; return await ConnectHostAsync() ?? throw new IOException(localError ?? "桌面服务不可用，请重新连接。"); }, () => PickFile(".zip"), UsePluginAsync,
            (projectId ?? model.ActiveProjectId) == model.ActiveProjectId ? model.ActiveSessionId : null, StartSpecialistChatAsync)
        { AppearanceModelFactory = GetAppearanceModel, PickCustomCss = () => PickFile(".css"), PickSkillSource = folder => folder ? PickDirectory() : PickFile(".md", ".zip"),
            WorkflowConversionFactory = GetWorkflowConversion, StorageBrowser = storageBrowser };
        settingsPage.ProjectRequested += id => _ = OpenSettingsProjectAsync(id);
        if (pageContent != null) pageContent.Visibility = Visibility.Collapsed;
        root.Children.Add(settingsPage);
        RefreshWorkflowConversionNotice();
    }

    private NativeLiveAppearanceModel GetAppearanceModel(NativeSettingsClient host, string? projectId)
    {
        // Keep a failed background save available for explicit retry after the
        // settings page has closed. Never reuse it across a different host.
        if (appearanceModelHost != host) { appearanceModels.Clear(); appearanceModelHost = host; }
        var key = projectId ?? "";
        if (!appearanceModels.TryGetValue(key, out var appearance))
            appearanceModels[key] = appearance = new(host, projectId, settings.ReadAppearanceCache);
        return appearance;
    }

    private void CloseSettings()
    {
        if (settingsPage == null) return;
        var appearanceSettled = appearancePersistence = settingsPage.AppearanceSettled;
        var generation = appearanceGeneration;
        var database = model.DatabasePath;
        root.Children.Remove(settingsPage);
        settingsPage.Dispose(); settingsPage = null;
        if (workspaceSheet is NativeAssistantPage retainedAssistant)
        {
            if (settingsAssistantProject != model.ActiveProjectId || settingsAssistantSession != model.ActiveSessionId) CloseSheet();
            else retainedAssistant.IsEnabled = true;
        }
        if (pageContent != null) pageContent.Visibility = Visibility.Visible;
        projectInventory.Select(database, model.ActiveProjectId, refresh: true);
        if (workspaceHost != null) _ = ReadInventoryAsync(workspaceHost);
        Render();
        _ = model.RefreshAsync();
        if (workspaceHost != null) _ = RefreshAppearanceAfterSettingsAsync(appearanceSettled, workspaceHost, generation, database);
    }

    private async Task RefreshAppearanceAfterSettingsAsync(Task<bool> settled, NativeSettingsClient host, int generation, string database)
    {
        if (await settled && !windowClosed && settingsPage == null && generation == appearanceGeneration && database == model.DatabasePath && host == workspaceHost)
            await ReadAppearanceAsync(host);
    }

    private async Task ReadInventoryAsync(INativeSettingsClient host)
    {
        if (await projectInventory.ReadAsync(host) && !windowClosed) RefreshInventoryLabel();
    }
    private void RefreshInventoryLabel()
    {
        if (inventoryLabel == null) return;
        var counts = projectInventory.Counts;
        inventoryLabel.Visibility = counts == null ? Visibility.Collapsed : Visibility.Visible;
        if (counts != null) inventoryLabel.Text = design.Localization.Format("sidebar.skills_meta",
            ("skills", counts.Skills.ToString()), ("mcp", counts.Mcp.ToString()), ("mem", counts.Memory.ToString()));
    }

    private async Task OpenSettingsProjectAsync(string id)
    {
        var owner = settingsPage; var database = model.DatabasePath;
        if (owner == null || windowClosed) return;
        // Joining a synced/Lab project returns an id that is not yet in the
        // read-only browser snapshot. OpenProjectAsync intentionally rejects it
        // until that list has been refreshed.
        if (!model.Projects.Any(project => project.Id == id)) await model.RefreshAsync();
        if (windowClosed || settingsPage != owner || database != model.DatabasePath) return;
        await model.OpenProjectAsync(id);
        if (!windowClosed && settingsPage == owner && database == model.DatabasePath && model.ActiveProjectId == id) CloseSettings();
    }

    private async Task UsePluginAsync(NativePluginLaunch launch)
    {
        var owner = settingsPage; var database = model.DatabasePath;
        if (owner == null || windowClosed) return;
        await model.OpenProjectAsync(launch.ProjectId, launch.SessionId);
        if (windowClosed || settingsPage != owner || database != model.DatabasePath || model.ActiveProjectId != launch.ProjectId || model.ActiveSessionId != launch.SessionId) return;
        await EnsureConversationAsync();
        if (windowClosed || settingsPage != owner || model.ActiveProjectId != launch.ProjectId || model.ActiveSessionId != launch.SessionId) return;
        if (conversation?.StagePluginLaunch(launch) != true)
            throw new InvalidOperationException("新会话已创建，但尚未准备好插件技能引用。未发送消息，请检查会话后重试。");
        CloseSettings();
        await conversation.SendAsync();
        conversationPage?.Refresh();
    }

    private async Task StartSpecialistChatAsync(NativeSpecialistChat launch)
    {
        var owner = settingsPage; var database = model.DatabasePath;
        if (owner == null || windowClosed) return;
        await model.OpenProjectAsync(launch.ProjectId, launch.SessionId);
        if (windowClosed || settingsPage != owner || database != model.DatabasePath || model.ActiveProjectId != launch.ProjectId || model.ActiveSessionId != launch.SessionId) return;
        await EnsureConversationAsync();
        if (windowClosed || settingsPage != owner || database != model.DatabasePath || model.ActiveProjectId != launch.ProjectId || model.ActiveSessionId != launch.SessionId) return;
        if (conversation?.StageSpecialistChat(launch) != true)
            throw new InvalidOperationException("新会话已创建，但尚未准备好。未发送消息，请检查会话后重试。");
        CloseSettings();
        await conversation.SendAsync();
        conversationPage?.Refresh();
    }

    private void ApplyAppearancePreferences(JsonObject prefs)
    {
        if (windowClosed) return;
        settings.CachedAppearance = NativeAppearanceSnapshot.ForCache(prefs);
        settings.Appearance = prefs["theme"]?.GetValue<string>() ?? "system";
        settings.LightPalette = prefs["light_palette"]?.GetValue<string>() ?? "paper";
        settings.DarkPalette = prefs["dark_palette"]?.GetValue<string>() ?? "charcoal";
        settings.Typography = NativeTypography.From(prefs);
        settings.InputPreferences = NativeInputPreferences.From(prefs);
        design.SelectionPopupEnabled = settings.InputPreferences.SelectionPopupEnabled;
        design.Typography = settings.Typography;
        conversation?.ApplyInputPreferences(settings.InputPreferences);
        SaveSettings();
        windowFrame.RequestedTheme = settings.Appearance switch { "light" => ElementTheme.Light, "dark" => ElementTheme.Dark, _ => ElementTheme.Default };
        Render();
    }

    private async Task ReadAppearanceAsync(NativeSettingsClient host)
    {
        var database = model.DatabasePath;
        var request = ++appearanceGeneration;
        using var deadline = new CancellationTokenSource(TimeSpan.FromSeconds(15));
        try
        {
            var editor = new AppearanceSettingsModel(host, null, settings.ReadAppearanceCache);
            await editor.LoadAsync(deadline.Token);
            var general = await host.InvokeAsync("get_settings", new(), null, deadline.Token);
            if (!windowClosed && request == appearanceGeneration && host == workspaceHost && database == model.DatabasePath && settingsPage == null)
            {
                design.Localization.ApplyCommittedSettings(general);
                syncBackendConfigured = HomeProjectSync.BackendConfigured(general);
                design.Welcome.ApplyCommittedSettings(general);
                conversationPage?.Refresh();
                ApplyAppearancePreferences(editor.Draft!);
            }
        }
        catch (Exception ex)
        {
            if (!windowClosed && request == appearanceGeneration && database == model.DatabasePath)
            { localError = "未能读取外观设置：" + ex.Message; Render(); }
        }
    }

    private Task ChangeAppearanceAsync(string theme) => ChangeAppearanceAsync((host, token) => AppearanceSettingsModel.ChangeThemeAsync(host, theme, token, settings.ReadAppearanceCache));
    private async Task ChangeAppearanceAsync(Func<NativeSettingsClient, CancellationToken, Task<JsonObject>> change)
    {
        if (changingAppearance || settingsPage != null) return;
        changingAppearance = true; appearanceGeneration++;
        var database = model.DatabasePath;
        Render();
        try
        {
            var host = await ConnectHostAsync() ?? throw new IOException("桌面服务不可用，请重新连接。");
            var request = ++appearanceGeneration;
            using var deadline = new CancellationTokenSource(TimeSpan.FromSeconds(15));
            var saved = await change(host, deadline.Token);
            if (!windowClosed && request == appearanceGeneration && database == model.DatabasePath)
            { localError = null; ApplyAppearancePreferences(saved); }
        }
        catch (Exception ex)
        {
            if (!windowClosed && database == model.DatabasePath)
                localError = "外观保存结果尚未确认；不会自动重试。请打开设置核对。" + ex.Message;
        }
        finally { changingAppearance = false; if (!windowClosed) Render(); }
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
    private FrameworkElement ErrorNotice(string error, string? detail = null)
    {
        var row = new Grid { ColumnSpacing = 8 };
        row.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
        row.ColumnDefinitions.Add(new() { Width = new GridLength(1, GridUnitType.Star) });
        row.Children.Add(design.Icon("circle-alert", 16));
        var label = Text(error + (detail == null ? "" : "\n" + detail), 12, "text");
        Microsoft.UI.Xaml.Automation.AutomationProperties.SetLiveSetting(label, Microsoft.UI.Xaml.Automation.Peers.AutomationLiveSetting.Polite);
        Grid.SetColumn(label, 1); row.Children.Add(label);
        return Card(row, 10);
    }
    private void UpdateSessionActivity(ConversationSnapshot snapshot)
    {
        if (windowClosed || snapshot.ActivityStatus is not { } status || !model.ApplySessionActivity(snapshot)) return;
        sessionTree?.UpdateActivity(snapshot.SessionId, status);
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
        if (quiet)
        {
            design.QuietButton(button);
            if (showLabel) { button.HorizontalAlignment = HorizontalAlignment.Stretch; button.Padding = new Thickness(10, 7, 10, 7); }
            if (primary) { button.Background = design.Brush("bg-elev"); button.BorderBrush = design.Brush("border"); button.BorderThickness = new Thickness(1); }
        }
        AutomationProperties.SetName(button, label); ToolTipService.SetToolTip(button, action == null ? label + "（只读预览，尚未接入）" : label);
        return button;
    }
}
