using System.Text.Json.Nodes;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Controls.Primitives;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Automation.Peers;
using Microsoft.UI.Xaml.Media;
using Microsoft.UI.Xaml.Documents;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

namespace Wisp.Science.Preview;

/// <summary>In-window settings page, pinned to the database/project until returning.</summary>
internal sealed class NativeSettingsPage : UserControl, IDisposable
{
    private readonly CancellationTokenSource lifetime = new();
    private readonly StackPanel form = new() { Spacing = 16 };
    private readonly ContentControl formHost = new() { HorizontalContentAlignment = HorizontalAlignment.Stretch };
    private readonly TextBlock status = new() { TextWrapping = TextWrapping.Wrap };
    private readonly StackPanel body = new() { Spacing = 20, Padding = new Thickness(24, 24, 24, 32), MaxWidth = 1040, HorizontalAlignment = HorizontalAlignment.Center };
    private readonly Grid shell = new();
    private readonly WispDesign design = new();
    internal WispDesign OverlayDesign => design;
    private readonly WispDesign previewDesign = new();
    private readonly StackPanel preview = new();
    private readonly Grid columns = new() { ColumnSpacing = 20, RowSpacing = 20 };
    private readonly ContentControl customTheme = new() { HorizontalContentAlignment = HorizontalAlignment.Stretch };
    private readonly Grid diffPreview = new();
    private readonly Border diffFrame = new() { BorderThickness = new Thickness(1), CornerRadius = new CornerRadius(12) };
    private readonly ProgressBar progress = new() { IsIndeterminate = true, Height = 3, Visibility = Visibility.Collapsed };
    private readonly Border previewCard = new() { Padding = new Thickness(24), CornerRadius = new CornerRadius(12), BorderThickness = new Thickness(1), VerticalAlignment = VerticalAlignment.Top };
    private readonly Button reload = new() { Visibility = Visibility.Collapsed };
    private readonly StackPanel appearanceError = new() { Spacing = 10, Padding = new Thickness(20, 8, 20, 12), HorizontalAlignment = HorizontalAlignment.Center, Visibility = Visibility.Collapsed };
    private readonly List<ComboBox> choices = [];
    private Action<string>? showTheme;
    private readonly SettingsTextMenus textMenus = new();
    private readonly Dictionary<string, (Border Swatch, TextBlock Value, Border Dot)> colorSamples = [];
    private readonly Action<JsonObject> apply;
    private readonly Action close;
    private bool started;
    private readonly string database;
    private readonly string? projectId;
    private NativeSettingsClient? client;
    private NativeLiveAppearanceModel? model;
    private bool closed;
    private bool working;
    public Task<bool> AppearanceSettled => model?.WaitForPersistenceAsync() ?? Task.FromResult(true);
    public Func<NativeSettingsClient, string?, NativeLiveAppearanceModel>? AppearanceModelFactory { private get; init; }
    public Func<Task<string?>>? PickCustomCss { private get; init; }
    public Func<bool,Task<string?>>? PickSkillSource { private get; init; }
    private TextBox? customCssEditor;
    private TextBlock? customCssBytes;
    private Button? clearCustomCss;
    private Button? importCustomCss;
    private bool updatingCustomCss, importingCustomCss, customCssExpanded;
    private double customCssHeight = 168;
    private long customCssRevision;
    private NativeSettingsSectionPage? capabilityPage;
    private readonly NativeWindowPreferences windowPreferences = new();
    public NativeStorageBrowser StorageBrowser { private get; init; } = new();
    private FrameworkElement? capabilityModal;
    private readonly Func<NativeSpecialistChat, Task>? startSpecialistChat;
    private NativeWorkflowStudio? workflowStudio;
    public Func<INativeSettingsClient, string?, NativeWorkflowConversionModel>? WorkflowConversionFactory { private get; init; }
    private readonly Dictionary<string, NativeWorkflowConversionModel> workflowConversions = [];
    public string? ScopeProject => settingsProject;
    public void ShowWorkflowConversion() => SelectSection("workflow-conversion");
    private NativeMemoryPage? memoryPage;
    private NativeHooksPage? hooksPage;
    private NativeSkillsPage? skillsPage;
    private NativeSkillStorePage? skillStorePage;
    private FrameworkElement? skillStoreModal;
    private NativePluginsPage? pluginsPage;
    private FrameworkElement? pluginsModal;
    private readonly Func<Task<string?>> pickPluginZip;
    private readonly Func<NativePluginLaunch, Task> usePlugin;
    public event Action<string>? ProjectRequested;
    private string skillsQuery = "", skillsFilter = "";
    private string? skillsProject;
    private FrameworkElement? navigationContent;
    private Action? resizeSettingsShell;
    private readonly Dictionary<string, Button> navigationButtons = [];
    private readonly List<Action> refreshLanguage = [];
    private readonly TextBox navigationSearch = new() { PlaceholderText = "搜索设置", Margin = new Thickness(0, 12, 0, 4) };
    private FrameworkElement? appearanceContent;
    private string section = "appearance";
    private string? pendingSection;
    private string? settingsProject;
    private readonly string? sessionId;
    private readonly ComboBox projectChoice = new() { Header = "项目作用域", HorizontalAlignment = HorizontalAlignment.Stretch };
    private bool changingProject;
    private readonly Func<bool, Task<string?>>? pickPath;
    private readonly Func<Task<NativeSettingsClient?>>? connectHost;
    private readonly StackPanel connectionContent = new() { Spacing = 16, Padding = new Thickness(28), Visibility = Visibility.Collapsed };
    private readonly TextBlock connectionHeading = new() { FontSize = 24 };
    private readonly TextBlock connectionStatus = new() { TextWrapping = TextWrapping.Wrap };
    private readonly Button connectionRetry = new() { Content = "重新连接", Visibility = Visibility.Collapsed };

    public NativeSettingsPage(string database, string? projectId, Action<JsonObject> apply, Action close, string initialSection = "appearance", IReadOnlyList<ProjectSummary>? projects = null, Func<bool, Task<string?>>? pickPath = null, NativeTypography? typography = null, Func<Task<NativeSettingsClient?>>? connectHost = null, Func<Task<string?>>? pickPluginZip = null, Func<NativePluginLaunch, Task>? usePlugin = null, string? sessionId = null, Func<NativeSpecialistChat, Task>? startSpecialistChat = null)
    {
        this.database = database; this.projectId = projectId; this.apply = apply; this.close = close;
        // Settings copy inherits the selected palette, just as it inherits
        // body { color: var(--text) } in WebView. Stateful controls keep their own brushes.
        Foreground = design.Brush("text");
        HorizontalContentAlignment = HorizontalAlignment.Stretch;
        VerticalContentAlignment = VerticalAlignment.Stretch;
        design.Typography = typography ?? new(); design.BindTypography(this);
        settingsProject = projectId;
        this.sessionId = sessionId;
        this.startSpecialistChat = startSpecialistChat;
        this.pickPath = pickPath;
        this.connectHost = connectHost;
        this.pickPluginZip = pickPluginZip ?? (() => Task.FromResult<string?>(null));
        this.usePlugin = usePlugin ?? (_ => Task.FromException(new InvalidOperationException("当前窗口未连接新会话入口。")));
        shell.ColumnDefinitions.Add(new() { Width = new GridLength(210) });
        shell.ColumnDefinitions.Add(new() { Width = new GridLength(1, GridUnitType.Star) });
        shell.RowDefinitions.Add(new() { Height = GridLength.Auto });
        shell.RowDefinitions.Add(new() { Height = new GridLength(1, GridUnitType.Star) });
        var nav = new StackPanel { Spacing = 14 };
        var backContent = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 8 };
        backContent.Children.Add(design.Icon("chevron-left", 16));
        var backLabel = NavigationText(design.T("settings.back_to_app"), 13.5); backContent.Children.Add(backLabel);
        var back = new Button { Content = backContent, HorizontalAlignment = HorizontalAlignment.Stretch,
            HorizontalContentAlignment = HorizontalAlignment.Left };
        design.QuietButton(back);
        back.Padding = new Thickness(14, 9, 14, 9); back.MinHeight = 0;
        back.BorderThickness = new Thickness(1); back.BorderBrush = new SolidColorBrush(Microsoft.UI.Colors.Transparent);
        back.Foreground = design.Brush("text-muted");
        AutomationProperties.SetName(back, "返回工作区");
        back.Click += (_, _) => RequestClose(); nav.Children.Add(back);
        var settingsHeading = NavigationText("设置", 20); settingsHeading.FontWeight = Microsoft.UI.Text.FontWeights.SemiBold; settingsHeading.Margin = new Thickness(14, 2, 0, 0); nav.Children.Add(settingsHeading);
        projectChoice.Items.Add(new ComboBoxItem { Content = "全局设置", Tag = "" });
        refreshLanguage.Add(() =>
        {
            backLabel.Text = design.T("settings.back_to_app"); AutomationProperties.SetName(back, backLabel.Text);
            settingsHeading.Text = design.T("settings.title");
            projectChoice.Header = design.L("项目作用域", "Project scope");
            ((ComboBoxItem)projectChoice.Items[0]).Content = design.L("全局设置", "Global settings");
            navigationSearch.PlaceholderText = design.T("settings.search");
            AutomationProperties.SetName(navigationSearch, design.T("settings.search"));
            connectionRetry.Content = design.L("重新连接", "Reconnect");
        });
        foreach (var project in projects ?? []) projectChoice.Items.Add(new ComboBoxItem { Content = project.Name, Tag = project.Id });
        projectChoice.SelectedItem = projectChoice.Items.Cast<ComboBoxItem>().FirstOrDefault(i => (string)i.Tag == (settingsProject ?? ""));
        projectChoice.SelectionChanged += (_, _) =>
        {
            if (changingProject || projectChoice.SelectedItem is not ComboBoxItem selected) return;
            var next = (string)selected.Tag;
            changingProject = true;
            projectChoice.SelectedItem = projectChoice.Items.Cast<ComboBoxItem>().FirstOrDefault(i => (string)i.Tag == (settingsProject ?? ""));
            changingProject = false;
            if (working || capabilityPage?.Busy == true) return;
            void Change()
            {
                settingsProject = next.Length == 0 ? null : next;
                changingProject = true; projectChoice.SelectedItem = selected; changingProject = false;
                if (capabilityPage != null) MountSection(section);
            }
            if (capabilityPage != null) capabilityPage.RequestLeave(Change, switchingCategory: true); else Change();
        };
        // Project scope belongs to project-specific categories, not global preferences.
        nav.Children.Add(projectChoice); projectChoice.Visibility = Visibility.Collapsed;
        navigationSearch.Margin = new Thickness(0); navigationSearch.Padding = new Thickness(11, 9, 11, 9);
        navigationSearch.CornerRadius = new CornerRadius(10);
        PreferenceFieldAppearance.StylePlaceholder(navigationSearch);
        navigationSearch.Background = design.Brush("bg-input"); navigationSearch.BorderBrush = design.Brush("border-strong");
        foreach (var state in new[] { "", "PointerOver", "Focused" })
        {
            navigationSearch.Resources["TextControlBackground" + state] = design.Brush("bg-input");
            navigationSearch.Resources["TextControlBorderBrush" + state] = design.Brush(state == "Focused" ? "clay" : "border-strong");
        }
        AutomationProperties.SetName(navigationSearch, "搜索设置分类与选项");
        textMenus.Register(navigationSearch);
        PreferenceFieldAppearance.Apply(navigationSearch, design);
        nav.Children.Add(navigationSearch);
        var navHeader = nav;
        nav = new StackPanel { Spacing = 20 };
        var navigation = JsonNode.Parse(File.ReadAllText(Path.Combine(AppContext.BaseDirectory, "Assets", "settings-navigation.json")))!.AsObject();
        var groupHeadings = new Dictionary<string, TextBlock>();
        var groupPanels = new Dictionary<string, StackPanel>();
        StackPanel? groupPanel = null;
        string? group = null;
        foreach (var entry in navigation)
        {
            var nextGroup = entry.Value!["group"]!.GetValue<string>();
            if (group != nextGroup)
            {
                group = nextGroup;
                groupPanel = new StackPanel { Spacing = 4 }; groupPanels[group] = groupPanel; nav.Children.Add(groupPanel);
                var groupHeading = NavigationText(group, 12); groupHeading.FontWeight = Microsoft.UI.Text.FontWeights.SemiBold; groupHeading.Foreground = design.Brush("text-muted");
                groupHeading.Margin = new Thickness(14, 0, 14, 4); groupHeading.CharacterSpacing = 40; groupPanel.Children.Add(groupHeading);
                groupHeadings[group] = groupHeading;
                var groupEntry = entry.Value!;
                refreshLanguage.Add(() => groupHeading.Text = design.Localization.Locale == "zh"
                    ? groupEntry["group"]!.GetValue<string>() : groupEntry["group_en"]!.GetValue<string>().ToUpperInvariant());
            }
            var itemLabel = NavigationText(entry.Value["zh"]!.GetValue<string>(), 13.5);
            // The stock button presenter can retain its last animated background
            // across a live palette change. Paint selection on our own surface.
            var plate = new Border { Child = itemLabel, CornerRadius = new CornerRadius(8),
                BorderThickness = new Thickness(1), Padding = new Thickness(14, 9, 14, 9) };
            var item = new Button { Content = plate,
                HorizontalAlignment = HorizontalAlignment.Stretch, HorizontalContentAlignment = HorizontalAlignment.Stretch,
                IsEnabled = entry.Key == "appearance" || NativeSettingsSectionPage.Titles.ContainsKey(entry.Key) };
            design.QuietButton(item); design.BindTypography(item, size: 13.5);
            item.Padding = new Thickness(0); item.MinHeight = 0;
            if (!item.IsEnabled) ToolTipService.SetToolTip(item, "此设置页尚未接入 Windows");
            var key = entry.Key;
            item.Click += (_, _) => SelectSection(key);
            navigationButtons[key] = item;
            var labels = entry.Value!;
            refreshLanguage.Add(() => { itemLabel.Text = labels[design.Localization.Locale]!.GetValue<string>(); AutomationProperties.SetName(item, itemLabel.Text); });
            groupPanel!.Children.Add(item);
        }
        var noMatches = NavigationText(design.T("settings.search_empty"), 13);
        noMatches.Foreground = design.Brush("text-muted"); noMatches.Margin = new Thickness(14, 0, 14, 0);
        noMatches.Visibility = Visibility.Collapsed;
        AutomationProperties.SetLiveSetting(noMatches, AutomationLiveSetting.Polite);
        refreshLanguage.Add(() => noMatches.Text = design.T("settings.search_empty"));
        nav.Children.Add(noMatches);
        void FilterNavigation()
        {
            var visibleGroups = new HashSet<string>();
            foreach (var entry in navigation)
            {
                var matches = NativeSettingsSearch.Matches(entry.Value!.AsObject(), navigationSearch.Text, design.Localization.Locale);
                navigationButtons[entry.Key].Visibility = matches ? Visibility.Visible : Visibility.Collapsed;
                if (matches) visibleGroups.Add(entry.Value["group"]!.GetValue<string>());
            }
            foreach (var (name, panel) in groupPanels) panel.Visibility = visibleGroups.Contains(name) ? Visibility.Visible : Visibility.Collapsed;
            var empty = visibleGroups.Count == 0;
            var announce = empty && noMatches.Visibility == Visibility.Collapsed;
            noMatches.Visibility = empty ? Visibility.Visible : Visibility.Collapsed;
            if (announce) FrameworkElementAutomationPeer.FromElement(noMatches)?.RaiseAutomationEvent(AutomationEvents.LiveRegionChanged);
        }
        navigationSearch.TextChanged += (_, _) => FilterNavigation();
        refreshLanguage.Add(FilterNavigation);
        // At fractional Windows scaling, rounding each row's padding/border
        // independently compounds into a visible offset down the navigation.
        var navigationPane = new Border { UseLayoutRounding = false, Background = design.Brush("bg-sunken"), BorderBrush = design.Brush("border"), BorderThickness = new Thickness(0, 0, 1, 0) };
        navigationContent = navigationPane;
        var navigationStack = new StackPanel { Spacing = 14, Padding = new Thickness(12, 18, 12, 18) };
        navigationStack.Children.Add(navHeader); navigationStack.Children.Add(nav);
        var navigationScroll = new ScrollViewer { Content = navigationStack, HorizontalScrollBarVisibility = ScrollBarVisibility.Disabled,
            HorizontalScrollMode = ScrollMode.Disabled, HorizontalContentAlignment = HorizontalAlignment.Stretch };
        navigationPane.Child = navigationScroll;
        void SizeSearchHint()
        {
            var occupied = navigationStack.Padding.Left + navHeader.ActualWidth + navigationStack.Spacing
                + noMatches.Margin.Left + noMatches.Margin.Right + navigationStack.Padding.Right;
            noMatches.MaxWidth = NativeSettingsLayout.SearchHintMaxWidth(shell.ActualWidth, occupied, noMatches.FontSize);
        }
        navHeader.SizeChanged += (_, _) => SizeSearchHint();
        noMatches.RegisterPropertyChangedCallback(TextBlock.FontSizeProperty, (_, _) => SizeSearchHint());
        void UpdateNavigationGutter()
        {
            var horizontal = NativeSettingsLayout.HorizontalNavigation(shell.ActualWidth);
            navigationStack.Margin = horizontal ? new Thickness(0, 0, 0, navigationScroll.ScrollableWidth > 0 ? 10 : 0)
                : new Thickness(0, 0, navigationScroll.ScrollableHeight > 0 ? 10 : 0, 0);
        }
        navigationScroll.RegisterPropertyChangedCallback(ScrollViewer.ScrollableHeightProperty, (_, _) => UpdateNavigationGutter());
        navigationScroll.RegisterPropertyChangedCallback(ScrollViewer.ScrollableWidthProperty, (_, _) => UpdateNavigationGutter());
        shell.Children.Add(navigationPane);
        var heading = new Grid { ColumnSpacing = 12, Padding = new Thickness(20, 16, 20, 14), HorizontalAlignment = HorizontalAlignment.Center,
            BorderBrush = design.Brush("border"), BorderThickness = new Thickness(0, 0, 0, 1) };
        heading.ColumnDefinitions.Add(new() { Width = new GridLength(1, GridUnitType.Star) });
        heading.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
        var appearanceHeading = design.Text("外观", 24);
        void SizeHeadingMargin()
        {
            var margin = NativeSettingsLayout.AppearanceHeadingMargin(appearanceHeading.FontSize);
            appearanceHeading.Margin = new Thickness(0, margin, 0, margin);
        }
        appearanceHeading.RegisterPropertyChangedCallback(TextBlock.FontSizeProperty, (_, _) => SizeHeadingMargin());
        SizeHeadingMargin();
        appearanceHeading.FontWeight = Microsoft.UI.Text.FontWeights.SemiBold; heading.Children.Add(appearanceHeading);
        reload.HorizontalAlignment = HorizontalAlignment.Left;
        design.ActionButton(reload);
        refreshLanguage.Add(() =>
        {
            appearanceHeading.Text = design.T("settings.nav.appearance");
            reload.Content = design.L("重试", "Retry");
        });
        body.Children.Add(progress);
        AutomationProperties.SetLiveSetting(status, Microsoft.UI.Xaml.Automation.Peers.AutomationLiveSetting.Polite);
        status.Foreground = design.Brush("err");
        appearanceError.Children.Add(status); appearanceError.Children.Add(reload);
        columns.ColumnDefinitions.Add(new() { Width = new GridLength(1.15, GridUnitType.Star) });
        columns.ColumnDefinitions.Add(new() { Width = new GridLength(1, GridUnitType.Star) });
        columns.RowDefinitions.Add(new() { Height = GridLength.Auto });
        columns.RowDefinitions.Add(new() { Height = GridLength.Auto });
        formHost.Content = form; columns.Children.Add(formHost);
        diffFrame.Child = diffPreview;
        previewCard.Child = preview; SizeAppearanceCard(previewCard); Grid.SetColumn(previewCard, 1); columns.Children.Add(previewCard);
        body.Children.Add(columns);
        body.Children.Add(customTheme);
        var scroll = new ScrollViewer { Content = body, HorizontalScrollBarVisibility = ScrollBarVisibility.Disabled, HorizontalScrollMode = ScrollMode.Disabled, HorizontalContentAlignment = HorizontalAlignment.Stretch };
        var previewOffset = new TranslateTransform(); previewCard.RenderTransform = previewOffset;
        void PositionPreview() => previewOffset.Y = NativeSettingsLayout.AppearanceColumns(shell.ActualWidth)
            ? Math.Clamp(scroll.VerticalOffset - body.Padding.Top, 0, Math.Max(0, columns.ActualHeight - previewCard.ActualHeight)) : 0;
        scroll.ViewChanged += (_, _) => PositionPreview(); previewCard.SizeChanged += (_, _) => PositionPreview();
        var appearance = new Grid();
        appearance.RowDefinitions.Add(new() { Height = GridLength.Auto });
        appearance.RowDefinitions.Add(new() { Height = new GridLength(1, GridUnitType.Star) });
        var appearanceHeader = new StackPanel(); appearanceHeader.Children.Add(heading); appearanceHeader.Children.Add(appearanceError);
        appearance.Children.Add(appearanceHeader); Grid.SetRow(scroll, 1); appearance.Children.Add(scroll);
        Grid.SetColumn(appearance, 1); shell.Children.Add(appearance); shell.Background = design.Brush("bg-app");
        appearanceContent = appearance;
        resizeSettingsShell = () =>
        {
            var horizontal = NativeSettingsLayout.HorizontalNavigation(shell.ActualWidth);
            var layout = NativeSettingsLayout.ForWidth(shell.ActualWidth);
            shell.ColumnDefinitions[0].Width = new GridLength(layout.NavigationWidth);
            foreach (var child in shell.Children.OfType<FrameworkElement>())
            {
                var full = child == workflowStudio || child == capabilityModal || child == skillStoreModal || child == pluginsModal;
                var isNav = child == navigationPane;
                Grid.SetColumn(child, full || horizontal || isNav ? 0 : 1);
                Grid.SetColumnSpan(child, full || horizontal ? 2 : 1);
                Grid.SetRow(child, horizontal && !full && !isNav ? 1 : 0);
                Grid.SetRowSpan(child, full || !horizontal ? 2 : 1);
            }
            var orientation = horizontal ? Orientation.Horizontal : Orientation.Vertical;
            navigationStack.Orientation = navHeader.Orientation = nav.Orientation = orientation;
            navigationStack.Padding = horizontal ? new Thickness(12, 10, 12, 10) : new Thickness(12, 18, 12, 18);
            SizeSearchHint();
            UpdateNavigationGutter();
            navigationStack.VerticalAlignment = horizontal ? VerticalAlignment.Center : VerticalAlignment.Stretch;
            navHeader.VerticalAlignment = nav.VerticalAlignment = horizontal ? VerticalAlignment.Center : VerticalAlignment.Stretch;
            nav.Spacing = horizontal ? 14 : 20;
            navigationPane.BorderThickness = horizontal ? new Thickness(0, 0, 0, 1) : new Thickness(0, 0, 1, 0);
            navigationScroll.HorizontalScrollBarVisibility = horizontal ? ScrollBarVisibility.Auto : ScrollBarVisibility.Disabled;
            navigationScroll.HorizontalScrollMode = horizontal ? ScrollMode.Enabled : ScrollMode.Disabled;
            navigationScroll.VerticalScrollBarVisibility = horizontal ? ScrollBarVisibility.Disabled : ScrollBarVisibility.Auto;
            navigationScroll.VerticalScrollMode = horizontal ? ScrollMode.Disabled : ScrollMode.Enabled;
            navigationSearch.Width = horizontal ? 170 : double.NaN;
            settingsHeading.Visibility = horizontal ? Visibility.Collapsed : Visibility.Visible;
            foreach (var panel in groupPanels.Values) panel.Orientation = orientation;
            foreach (var label in groupHeadings.Values) label.Visibility = horizontal ? Visibility.Collapsed : Visibility.Visible;
            heading.Width = appearanceError.Width = layout.ContentWidth;
            body.Width = NativeSettingsLayout.AppearanceBodyWidth(shell.ActualWidth);
            body.Margin = new Thickness(0, 0, 10, 0);
        };
        connectionContent.Children.Add(connectionHeading);
        connectionContent.Children.Add(connectionStatus);
        connectionContent.Children.Add(connectionRetry);
        connectionRetry.Click += async (_, _) => await LoadAsync(reconnect: true);
        Grid.SetColumn(connectionContent, 1); shell.Children.Add(connectionContent);
        pendingSection = initialSection;
        ShowConnectingSection(initialSection);
        Content = shell;
        shell.SizeChanged += (_, e) =>
        {
            resizeSettingsShell();
            var narrow = !NativeSettingsLayout.AppearanceColumns(e.NewSize.Width);
            body.Padding = e.NewSize.Width <= 850 ? new Thickness(16) : new Thickness(24, 24, 24, 32);
            columns.ColumnSpacing = narrow ? 0 : 20;
            columns.ColumnDefinitions[1].Width = narrow ? new GridLength(0) : new GridLength(1, GridUnitType.Star);
            Grid.SetColumn(previewCard, narrow ? 0 : 1); Grid.SetRow(previewCard, narrow ? 1 : 0);
            LayoutDiffPreview();
            PositionPreview();
        };
        RefreshLanguage(); RenderForm();
        reload.Click += async (_, _) => { if (model?.Error != null) model.Retry(); else await LoadAsync(reconnect: true); };
        ActualThemeChanged += (_, _) => { if (!closed && model?.Draft is { } prefs) RefreshAppearanceVisuals(prefs); };
        Loaded += async (_, _) => { if (!started) { started = true; await LoadAsync(); } };
    }

    public void HandleEscape()
    {
        if (textMenus.CloseTop()) return;
        if (workflowStudio != null) { workflowStudio.HandleEscape(); return; }
        if (memoryPage != null) { memoryPage.HandleEscape(); return; }
        if (hooksPage != null) { hooksPage.HandleEscape(); return; }
        if (skillsPage != null) { skillsPage.HandleEscape(); return; }
        if (skillStorePage != null) { skillStorePage.HandleEscape(); return; }
        if (pluginsPage != null) { pluginsPage.HandleEscape(); return; }
        if (projectChoice.IsDropDownOpen) projectChoice.IsDropDownOpen = false;
        else if (capabilityPage != null) capabilityPage.HandleEscape();
        else if (choices.LastOrDefault(c => c.IsDropDownOpen) is { } choice) choice.IsDropDownOpen = false;
        else RequestClose();
    }

    private void RequestClose() => RequestLeave(close);
    internal void RequestLeave(Action leave)
    {
        if (workflowStudio != null) { workflowStudio.RequestLeave(leave); return; }
        if (memoryPage != null) { memoryPage.RequestLeave(leave); return; }
        if (hooksPage != null) { hooksPage.RequestLeave(leave); return; }
        if (skillsPage != null) { skillsPage.RequestLeave(leave); return; }
        if (skillStorePage != null) { skillStorePage.RequestLeave(leave); return; }
        if (pluginsPage != null) { pluginsPage.RequestLeave(leave); return; }
        if (capabilityPage != null) { capabilityPage.RequestLeave(leave, closingSettings: true); return; }
        leave();
    }

    internal void SelectSection(string next)
    {
        if (closed) return;
        if (pendingSection != null)
        {
            pendingSection = next;
            var previousStatus = connectionStatus.Text;
            ShowConnectingSection(next);
            if (connectionRetry.Visibility == Visibility.Visible) connectionStatus.Text = previousStatus;
            return;
        }
        if (working) return;
        if (next == section)
        {
            ((NativeActionPage?)capabilityPage ?? memoryPage)?.ReselectSettingsCategory();
            return;
        }
        if (workflowStudio != null) { workflowStudio.RequestLeave(() => MountSection(next)); return; }
        if (memoryPage != null) { memoryPage.RequestLeave(() => MountSection(next)); return; }
        if (hooksPage != null) { hooksPage.RequestLeave(() => MountSection(next)); return; }
        if (skillsPage != null) { skillsPage.RequestLeave(() => MountSection(next)); return; }
        if (skillStorePage != null) { skillStorePage.RequestLeave(() => MountSection(next)); return; }
        if (pluginsPage != null) { pluginsPage.RequestLeave(() => MountSection(next)); return; }
        if (capabilityPage != null) { capabilityPage.RequestLeave(() => MountSection(next), switchingCategory: true); return; }
        MountSection(next);
    }
    private void ShowSkillModal(FrameworkElement? modal)
    {
        if (skillStoreModal != null) shell.Children.Remove(skillStoreModal);
        skillStoreModal = modal;
        if (modal != null) { Grid.SetColumnSpan(modal, 2); shell.Children.Add(modal); }
        resizeSettingsShell?.Invoke();
    }
    private void MountSection(string next, string? workflowId = null, string? specialistId = null)
    {
        var openConversion = next == "workflow-conversion";
        if (openConversion) next = "workflows";
        var previous = section;
        connectionContent.Visibility = Visibility.Collapsed;
        if (capabilityPage != null) { shell.Children.Remove(capabilityPage); capabilityPage.Dispose(); capabilityPage = null; }
        if (workflowStudio != null) { shell.Children.Remove(workflowStudio); workflowStudio.Dispose(); workflowStudio = null; }
        if (memoryPage != null) { shell.Children.Remove(memoryPage); memoryPage.Dispose(); memoryPage = null; }
        if (hooksPage != null) { shell.Children.Remove(hooksPage); hooksPage.Dispose(); hooksPage = null; }
        if (skillStorePage != null) { shell.Children.Remove(skillStorePage); skillStorePage.Dispose(); skillStorePage = null; }
        if (pluginsPage != null) { shell.Children.Remove(pluginsPage); pluginsPage.Dispose(); pluginsPage = null; }
        if (skillsPage != null)
        {
            skillsQuery = skillsPage.Query; skillsFilter = skillsPage.Filter; skillsProject = skillsPage.ProjectId;
            shell.Children.Remove(skillsPage); skillsPage.Dispose(); skillsPage = null;
        }
        if (navigationContent != null) navigationContent.Visibility = next == "workflows" ? Visibility.Collapsed : Visibility.Visible;
        section = next; pendingSection = null;
        projectChoice.Visibility = NativeSettingsLayout.ShowsProjectScope(next) ? Visibility.Visible : Visibility.Collapsed;
        if (appearanceContent != null) appearanceContent.Visibility = next == "appearance" ? Visibility.Visible : Visibility.Collapsed;
        if (next == "workflows" && client != null)
        {
            var owner = settingsProject ?? "";
            if (!workflowConversions.TryGetValue(owner, out var conversion)) workflowConversions[owner] = conversion = WorkflowConversionFactory?.Invoke(client, settingsProject) ?? new(owner);
            workflowStudio = new NativeWorkflowStudio(client, settingsProject, design, () => MountSection("quick-actions"), () => MountSection("workflow-conversion"), workflowId)
            { ConversionState = conversion, OpenConversionInitially = openConversion };
            Grid.SetColumnSpan(workflowStudio, 2); shell.Children.Add(workflowStudio);
        }
        else if (next == "memory" && client != null)
        {
            memoryPage = new NativeMemoryPage(client, settingsProject, design, close);
            Grid.SetColumn(memoryPage, 1); shell.Children.Add(memoryPage);
        }
        else if (next == "hooks" && client != null)
        {
            hooksPage = new NativeHooksPage(client, settingsProject, design, close, () => MountSection("specialists", specialistId: "reviewer"));
            Grid.SetColumn(hooksPage, 1); shell.Children.Add(hooksPage);
        }
        else if (next == "skills" && client != null)
        {
            skillsPage = new NativeSkillsPage(client, settingsProject, design, close, github => MountSection(github ? "skill-github" : "skill-store"), PickSkillSource ?? pickPath, skillsQuery, skillsFilter, skillsProject);
            skillsPage.ModalChanged += ShowSkillModal;
            Grid.SetColumn(skillsPage, 1); shell.Children.Add(skillsPage);
        }
        else if (next is "skill-store" or "skill-github" && client != null)
        {
            skillStorePage = new NativeSkillStorePage(client, skillsProject, next == "skill-github", design, () => MountSection("skills"));
            skillStorePage.ModalChanged += ShowSkillModal;
            Grid.SetColumn(skillStorePage, 1); shell.Children.Add(skillStorePage);
        }
        else if (next == "plugins" && client != null)
        {
            pluginsPage = new NativePluginsPage(client, settingsProject, design, close, pickPluginZip, usePlugin);
            pluginsPage.ModalChanged += modal =>
            {
                if (pluginsModal != null) shell.Children.Remove(pluginsModal);
                pluginsModal = modal;
                if (modal != null) { Grid.SetColumnSpan(modal, 2); shell.Children.Add(modal); }
                resizeSettingsShell?.Invoke();
            };
            Grid.SetColumn(pluginsPage, 1); shell.Children.Add(pluginsPage);
        }
        else if (next != "appearance" && client != null)
        {
            capabilityPage = new NativeSettingsSectionPage(client, settingsProject, next, design, next == "workflow-conversion" ? () => MountSection("workflows") : close, pickPath,
                settingsProject == projectId ? sessionId : null, next == "general" ? model : null, ApplyAppearance)
            { StorageBrowser = StorageBrowser, WindowPreferences = windowPreferences };
            capabilityPage.WorkflowRequested += id => MountSection("workflows", id);
            capabilityPage.ProjectRequested += id => ProjectRequested?.Invoke(id);
            capabilityPage.LanguageChanged += RefreshLanguage;
            capabilityPage.StartSpecialistChat = startSpecialistChat;
            capabilityPage.ModalChanged += modal =>
            {
                if (capabilityModal != null) shell.Children.Remove(capabilityModal);
                capabilityModal = modal;
                if (modal != null)
                {
                    Grid.SetColumnSpan(modal, 2); shell.Children.Add(modal);
                }
                resizeSettingsShell?.Invoke();
            };
            if (specialistId != null) capabilityPage.OpenSpecialist(specialistId);
            Grid.SetColumn(capabilityPage, 1); shell.Children.Add(capabilityPage);
        }
        foreach (var (key, button) in navigationButtons)
        {
            StyleNavigation(button, key == next || key == "skills" && next is "skill-store" or "skill-github");
        }
        resizeSettingsShell?.Invoke();
        // Refresh externally changed preferences when returning to Appearance.
        if (next == "appearance" && previous != "appearance") _ = LoadAsync();
    }

    private void ShowConnectingSection(string next)
    {
        section = next;
        connectionHeading.Text = NativeSettingsSectionPage.SectionTitle(design, next);
        connectionStatus.Text = design.L("正在连接桌面服务…可以切换分类或返回。", "Connecting to the desktop service… You can switch sections or go back.");
        connectionContent.Visibility = Visibility.Visible;
        if (appearanceContent != null) appearanceContent.Visibility = Visibility.Collapsed;
        foreach (var (key, button) in navigationButtons)
        {
            StyleNavigation(button, key == next);
        }
    }

    private void StyleNavigation(Button button, bool selected)
    {
        var plate = (Border)button.Content;
        plate.Background = selected ? design.Brush("bg-elev") : null;
        plate.BorderBrush = selected ? design.Brush("border") : null;
        button.FontWeight = selected ? Microsoft.UI.Text.FontWeights.SemiBold : Microsoft.UI.Text.FontWeights.Normal;
        button.Foreground = design.Brush(selected ? "text" : "text-muted");
        if (plate.Child is TextBlock label) { label.FontWeight = button.FontWeight; label.Foreground = button.Foreground; }
    }

    private void RefreshLanguage()
    {
        foreach (var update in refreshLanguage) update();
        connectionHeading.Text = NativeSettingsSectionPage.SectionTitle(design, section);
        Language = design.Localization.Locale == "zh" ? "zh-Hans" : "en-US";
    }

    public void Dispose()
    {
        if (closed) return;
        closed = true; textMenus.Dispose(); capabilityPage?.Dispose(); workflowStudio?.Dispose(); memoryPage?.Dispose(); hooksPage?.Dispose(); skillsPage?.Dispose(); skillStorePage?.Dispose(); pluginsPage?.Dispose(); lifetime.Cancel(); lifetime.Dispose();
        windowPreferences.Dispose();
        if (model != null) model.Changed -= RefreshAppearanceStatus;
        if (connectHost == null && client != null) _ = ReleaseClientAfterAppearanceAsync(client, AppearanceSettled);
    }

    private static async Task ReleaseClientAfterAppearanceAsync(NativeSettingsClient ownedClient, Task<bool> settled)
    { try { await settled; } finally { ownedClient.Dispose(); } }

    private void SetBusy(bool value)
    {
        working = value; progress.Visibility = value ? Visibility.Visible : Visibility.Collapsed;
        formHost.IsEnabled = !value && model?.Draft is not null;
        customTheme.IsEnabled = !value && model?.Draft is not null;
        foreach (var control in form.Children.OfType<Control>()) control.IsEnabled = !value && model?.Draft is not null;
        reload.IsEnabled = !value;
    }

    private async Task LoadAsync(bool reconnect = false)
    {
        if (working) return;
        SetBusy(true); appearanceError.Visibility = Visibility.Collapsed;
        connectionRetry.Visibility = Visibility.Collapsed;
        using var deadline = CancellationTokenSource.CreateLinkedTokenSource(lifetime.Token);
        deadline.CancelAfter(TimeSpan.FromSeconds(15));
        bool generalLoaded = false;
        try
        {
            if (pendingSection is { } loading) ShowConnectingSection(loading);
            if (reconnect && client != null) await client.ReconnectAsync(deadline.Token);
            if (client is null)
            {
                string? host = Environment.GetEnvironmentVariable("WISP_SETTINGS_HOST_PATH")
                    ?? Path.Combine(AppContext.BaseDirectory, "settings-host", "wisp-tauri.exe");
                var defaultDatabase = Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.ApplicationData),
                    "science.wisp-science", "wisp-science", "wisp.sqlite");
                if (!string.Equals(Path.GetFullPath(database), Path.GetFullPath(defaultDatabase), StringComparison.OrdinalIgnoreCase))
                    host = null; // Never start a default-database host on behalf of an alternate database.
                var connected = connectHost != null ? await connectHost() : await NativeSettingsClient.ConnectAsync(database, host, deadline.Token);
                if (closed) { if (connectHost == null) connected?.Dispose(); return; }
                if (connected == null) throw new IOException("桌面服务不可用。请检查完整安装包后重新连接。");
                client = connected;
                model = AppearanceModelFactory?.Invoke(client, projectId) ?? new(client, projectId);
                model.Changed += RefreshAppearanceStatus;
            }
            deadline.CancelAfter(TimeSpan.FromSeconds(15));
            var general = await client!.InvokeAsync("get_settings", new(), projectId, deadline.Token);
            if (closed) return;
            generalLoaded = true;
            if (windowPreferences.PreviewLocale is { } locale) design.Localization.Preview(locale);
            else design.Localization.ApplyCommittedSettings(general);
            RefreshLanguage();
            await model!.LoadAsync(deadline.Token);
            if (closed) return;
            if (model.Draft is { } loaded) ApplyAppearance(loaded);
            RenderForm(); RefreshAppearanceStatus();
        }
        catch (Exception ex)
        {
            if (!closed)
            {
                status.Text = design.L("未能读取设置。", "Could not load settings.") + "\n" + ex.Message;
                connectionStatus.Text = status.Text;
                appearanceError.Visibility = reload.Visibility = connectionRetry.Visibility = Visibility.Visible;
            }
        }
        finally
        {
            if (!closed)
            {
                SetBusy(false);
                if (generalLoaded && pendingSection is { } next) MountSection(next);
            }
        }
    }

    private void RenderCustomTheme(JsonObject prefs)
    {
        var content = new StackPanel();
        var disclosureContent = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 10 };
        var arrow = new ContentControl { Content = design.Icon(customCssExpanded ? "chevron-down" : "chevron-right", 12), VerticalAlignment = VerticalAlignment.Center };
        disclosureContent.Children.Add(arrow);
        var label = Heading(design.T("appearance.custom_css")); disclosureContent.Children.Add(label);
        var format = design.Text("CSS", 12); format.Foreground = design.Brush("text-faint"); format.VerticalAlignment = VerticalAlignment.Center;
        disclosureContent.Children.Add(format);
        var disclosure = new Button { Content = disclosureContent, HorizontalAlignment = HorizontalAlignment.Stretch, HorizontalContentAlignment = HorizontalAlignment.Left };
        design.QuietButton(disclosure); disclosure.Padding = new Thickness(24, 18, 24, 18);
        AutomationProperties.SetName(disclosure, design.T("appearance.custom_css"));
        AutomationProperties.SetAutomationId(disclosure, "appearance-custom-css-summary");
        content.Children.Add(disclosure);
        var expanded = new StackPanel { Visibility = customCssExpanded ? Visibility.Visible : Visibility.Collapsed };
        disclosure.Click += (_, _) =>
        {
            customCssExpanded = !customCssExpanded;
            expanded.Visibility = customCssExpanded ? Visibility.Visible : Visibility.Collapsed;
            arrow.Content = design.Icon(customCssExpanded ? "chevron-down" : "chevron-right", 12);
        };
        var head = new StackPanel { Spacing = 10, Padding = new Thickness(24, 12, 24, 14),
            BorderBrush = design.Brush("border"), BorderThickness = new Thickness(0, 1, 0, 1) };
        var row = new Grid { ColumnSpacing = 16, RowSpacing = 8 };
        row.ColumnDefinitions.Add(new() { Width = new GridLength(1, GridUnitType.Star) }); row.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
        row.RowDefinitions.Add(new() { Height = GridLength.Auto }); row.RowDefinitions.Add(new() { Height = GridLength.Auto });
        var title = Heading(design.T("appearance.custom_css")); title.VerticalAlignment = VerticalAlignment.Center; row.Children.Add(title);
        var actions = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 8, HorizontalAlignment = HorizontalAlignment.Right };
        var importContent = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 7 };
        importContent.Children.Add(design.Icon("upload", 16)); importContent.Children.Add(design.Text(design.T("appearance.custom_css_import"), 13.5));
        var import = importCustomCss = new Button { Content = importContent, IsEnabled = PickCustomCss != null && !importingCustomCss };
        design.ActionButton(import); import.Padding = new Thickness(14, 8, 14, 8);
        AutomationProperties.SetName(import, design.T("appearance.custom_css_import"));
        AutomationProperties.SetAutomationId(import, "appearance-custom-css-import");
        clearCustomCss = new Button { Content = design.T("appearance.custom_css_clear") };
        design.ActionButton(clearCustomCss); design.BindTypography(clearCustomCss, size: 13.5); clearCustomCss.Padding = new Thickness(14, 8, 14, 8);
        AutomationProperties.SetAutomationId(clearCustomCss, "appearance-custom-css-clear");
        actions.Children.Add(import); actions.Children.Add(clearCustomCss); Grid.SetColumn(actions, 1); row.Children.Add(actions); head.Children.Add(row);
        var meta = new Grid { ColumnSpacing = 24, RowSpacing = 8 };
        meta.ColumnDefinitions.Add(new() { Width = new GridLength(1, GridUnitType.Star) }); meta.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
        meta.RowDefinitions.Add(new() { Height = GridLength.Auto }); meta.RowDefinitions.Add(new() { Height = GridLength.Auto });
        var hint = design.Text(design.T("appearance.custom_css_hint"), 12); hint.Foreground = design.Brush("text-faint"); meta.Children.Add(hint);
        var byteCount = customCssBytes = design.Text("", 12); byteCount.Foreground = design.Brush("text-faint"); byteCount.TextWrapping = TextWrapping.NoWrap;
        AutomationProperties.SetAutomationId(byteCount, "appearance-custom-css-status");
        Grid.SetColumn(byteCount, 1); meta.Children.Add(byteCount); head.Children.Add(meta);
        var platformHint = design.Text(design.L("此 CSS 应用于 WebView 客户端；原生控件使用上方的主题和字体设置。", "This CSS applies to WebView clients; native controls use the theme and font settings above."), 12);
        platformHint.Foreground = design.Brush("text-faint"); head.Children.Add(platformHint);
        var importError = design.Text("", 12); importError.Foreground = design.Brush("err"); importError.Visibility = Visibility.Collapsed;
        AutomationProperties.SetLiveSetting(importError, Microsoft.UI.Xaml.Automation.Peers.AutomationLiveSetting.Polite); head.Children.Add(importError);
        head.SizeChanged += (_, e) =>
        {
            var compact = shell.ActualWidth <= 680 || e.NewSize.Width < 430;
            Grid.SetColumn(actions, compact ? 0 : 1); Grid.SetRow(actions, compact ? 1 : 0);
            actions.HorizontalAlignment = compact ? HorizontalAlignment.Left : HorizontalAlignment.Right;
            Grid.SetColumnSpan(title, compact ? 2 : 1);
            Grid.SetColumn(byteCount, compact ? 0 : 1); Grid.SetRow(byteCount, compact ? 1 : 0);
            Grid.SetColumnSpan(hint, compact ? 2 : 1);
        };
        expanded.Children.Add(head);
        var css = customCssEditor = new TextBox { AcceptsReturn = true, Text = prefs["custom_css"]?.GetValue<string>() ?? "", TextWrapping = TextWrapping.Wrap,
            Height = customCssHeight, MinHeight = 168, Padding = new Thickness(24, 16, 24, 20), BorderThickness = new Thickness(0), CornerRadius = new CornerRadius(0, 0, 12, 12),
            Background = design.Brush("bg-sunken"), Foreground = design.Brush("text"), IsSpellCheckEnabled = false, IsTextPredictionEnabled = false };
        design.BindTypography(css, code: true);
        css.Resources["TextControlBackgroundPointerOver"] = css.Resources["TextControlBackgroundFocused"] = design.Brush("bg-sunken");
        css.Resources["TextControlBorderBrushFocused"] = design.Brush("clay");
        css.Resources["TextControlBorderThemeThicknessFocused"] = new Thickness(2);
        css.Resources["TextControlForegroundFocused"] = design.Brush("text");
        AutomationProperties.SetName(css, design.T("appearance.custom_css")); AutomationProperties.SetAutomationId(css, "appearance-custom-css");
        ScrollViewer.SetVerticalScrollBarVisibility(css, ScrollBarVisibility.Auto); textMenus.Register(css);
        css.TextChanged += (_, _) =>
        {
            if (updatingCustomCss || customCssEditor != css) return;
            customCssRevision++; ChangeAppearance("custom_css", JsonValue.Create(css.Text)); RefreshCustomCss();
        };
        clearCustomCss.Click += (_, _) => { css.Text = ""; css.Focus(FocusState.Programmatic); };
        import.Click += async (_, _) =>
        {
            if (importingCustomCss || PickCustomCss == null) return;
            var revision = customCssRevision; importingCustomCss = true; import.IsEnabled = false; importError.Visibility = Visibility.Collapsed;
            try
            {
                var path = await PickCustomCss();
                if (path == null || closed) return;
                var imported = await NativeCustomTheme.ReadFileAsync(path, lifetime.Token);
                if (!closed && section == "appearance" && customCssEditor == css && revision == customCssRevision) css.Text = imported;
            }
            catch (OperationCanceledException) when (closed) { }
            catch (Exception ex) { if (!closed && customCssEditor == css) { importError.Text = NativeBrowserPresentation.ConnectionError(ex); importError.Visibility = Visibility.Visible; } }
            finally { importingCustomCss = false; if (!closed && importCustomCss != null) importCustomCss.IsEnabled = PickCustomCss != null; }
        };
        var editor = new Grid(); editor.Children.Add(css);
        var resize = new Microsoft.UI.Xaml.Controls.Primitives.Thumb { Width = 16, Height = 16, HorizontalAlignment = HorizontalAlignment.Right,
            VerticalAlignment = VerticalAlignment.Bottom, Background = new SolidColorBrush(Microsoft.UI.Colors.Transparent), IsTabStop = true };
        var grip = design.Icon("expand", 12); grip.HorizontalAlignment = HorizontalAlignment.Right; grip.VerticalAlignment = VerticalAlignment.Bottom; grip.Margin = new Thickness(2); grip.Opacity = .45; grip.IsHitTestVisible = false;
        editor.Children.Add(grip); editor.Children.Add(resize);
        AutomationProperties.SetName(resize, design.L("调整 CSS 编辑区高度，上下方向键调整", "Resize CSS editor; use Up and Down arrows"));
        resize.PointerPressed += (_, _) => resize.Focus(FocusState.Pointer);
        resize.DragDelta += (_, e) => css.Height = customCssHeight = Math.Clamp(customCssHeight + e.VerticalChange, 168, 1200);
        resize.KeyDown += (_, e) =>
        {
            if (e.Key is not (Windows.System.VirtualKey.Up or Windows.System.VirtualKey.Down)) return;
            css.Height = customCssHeight = Math.Clamp(customCssHeight + (e.Key == Windows.System.VirtualKey.Up ? -24 : 24), 168, 1200); e.Handled = true;
        };
        expanded.Children.Add(editor); content.Children.Add(expanded);
        var card = design.Card(content); card.Padding = new Thickness(0); customTheme.Content = card;
        RefreshCustomCss();
    }

    private void RefreshCustomCss()
    {
        if (customCssEditor == null || model?.Draft == null) return;
        var css = model.Draft["custom_css"]?.GetValue<string>() ?? "";
        if (customCssEditor.Text != css)
        {
            var caret = customCssEditor.SelectionStart; var length = customCssEditor.SelectionLength;
            updatingCustomCss = true;
            try { customCssEditor.Text = css; customCssEditor.Select(Math.Min(caret, css.Length), Math.Min(length, Math.Max(0, css.Length - caret))); }
            finally { updatingCustomCss = false; }
        }
        if (customCssBytes != null) customCssBytes.Text = design.Localization.Format("appearance.custom_css_bytes", ("n", NativeCustomTheme.ByteCount(css).ToString(System.Globalization.CultureInfo.InvariantCulture)));
        if (clearCustomCss != null) clearCustomCss.IsEnabled = css.Length > 0;
    }

    private void RefreshAppearanceStatus()
    {
        if (closed) return;
        RefreshCustomCss();
        appearanceError.Visibility = reload.Visibility = model?.Error != null ? Visibility.Visible : Visibility.Collapsed;
        status.Text = model?.Error is { } error
            ? design.L("外观或交互设置已应用，但保存未确认。修改已保留；点击重试以再次保存。", "Appearance or interaction settings are applied, but saving was not confirmed. Changes are retained; choose Retry to save again. ") + error : "";
        reload.IsEnabled = !working && model?.Saving != true;
    }

    private void ChangeAppearance(string key, JsonNode? value)
    {
        if (closed || working || model?.Draft == null) return;
        model.Set(key, value);
        ApplyAppearance(model.Draft);
    }

    private void ApplyAppearance(JsonObject preferences)
    {
        RequestedTheme = preferences["theme"]?.GetValue<string>() switch { "light" => ElementTheme.Light, "dark" => ElementTheme.Dark, _ => ElementTheme.Default };
        RefreshAppearanceVisuals(preferences);
        apply(preferences);
    }

    private void RefreshAppearanceVisuals(JsonObject preferences)
    {
        design.Typography = NativeTypography.From(preferences);
        design.Dark = preferences["theme"]?.GetValue<string>() == "dark" ||
            (preferences["theme"]?.GetValue<string>() == "system" && shell.ActualTheme == ElementTheme.Dark);
        design.LightPalette = preferences["light_palette"]?.GetValue<string>() ?? "paper";
        design.DarkPalette = preferences["dark_palette"]?.GetValue<string>() ?? "charcoal";
        design.RefreshPalette();
        showTheme?.Invoke(preferences["theme"]?.GetValue<string>() ?? "system");
        foreach (var sample in colorSamples.Values) sample.Value.FontFamily = design.CodeFont;
        design.ApplyTypography(shell, previewCard);
        PreferenceFieldAppearance.Apply(navigationSearch, design);
        PreferenceFieldAppearance.ApplyTree(form, design);
        UpdatePreview(preferences);
    }
    private void RenderForm()
    {
        showTheme = null; choices.Clear(); colorSamples.Clear(); form.Children.Clear();
        var loaded = model?.Draft is not null;
        var prefs = model?.Draft ?? new JsonObject { ["theme"] = "system", ["light_palette"] = "paper", ["dark_palette"] = "charcoal", ["ui_font_size"] = 14, ["code_font_size"] = 12 };
        var target = new StackPanel();
        var modes = new Grid { ColumnSpacing = 10 };
        AutomationProperties.SetName(modes, design.T("appearance.theme"));
        var themeCards = new List<Action<string>>();
        var clear = Solid("#000000", 0);
        foreach (var (value, label) in new[] { ("system", design.T("appearance.system")), ("light", design.T("appearance.light")), ("dark", design.T("appearance.dark")) })
        {
            modes.ColumnDefinitions.Add(new() { Width = new GridLength(1, GridUnitType.Star) });
            var sample = ThemeSample(value);
            var content = new StackPanel { Spacing = 9 }; content.Children.Add(sample);
            var caption = design.Text(label, 14); caption.FontWeight = Microsoft.UI.Text.FontWeights.Medium; caption.HorizontalAlignment = HorizontalAlignment.Center; content.Children.Add(caption);
            var button = new ToggleButton { Content = content, Padding = new Thickness(0), HorizontalAlignment = HorizontalAlignment.Stretch,
                HorizontalContentAlignment = HorizontalAlignment.Stretch, Background = clear, BorderThickness = new Thickness(0) };
            AutomationProperties.SetName(button, label);
            AutomationProperties.SetAutomationId(button, "theme-mode-" + value);
            foreach (var state in new[] { "PointerOver", "Pressed", "Checked", "CheckedPointerOver", "CheckedPressed" })
                button.Resources["ToggleButtonBackground" + state] = clear;
            bool hovered = false;
            void ColorCard()
            {
                caption.Foreground = design.Brush(hovered ? "text" : "text-muted");
                sample.BorderBrush = button.IsChecked == true ? design.Brush("text-muted") : hovered ? design.Brush("text-faint") : clear;
            }
            button.PointerEntered += (_, _) => { hovered = true; ColorCard(); };
            button.PointerExited += (_, _) => { hovered = false; ColorCard(); };
            themeCards.Add(mode => { button.IsChecked = mode == value; ColorCard(); });
            button.Click += (_, _) => ChangeAppearance("theme", JsonValue.Create(value));
            Grid.SetColumn(button, modes.ColumnDefinitions.Count - 1); modes.Children.Add(button);
        }
        var themeGroup = new StackPanel { Spacing = 16 }; themeGroup.Children.Add(Heading(design.T("appearance.theme"), 15)); themeGroup.Children.Add(modes);
        var themeCard = design.Card(themeGroup); SizeAppearanceCard(themeCard); form.Children.Add(themeCard);
        var paletteCard = design.Card(target); paletteCard.Padding = new Thickness(20, 4, 20, 4); form.Children.Add(paletteCard);
        // WebView edits the light palette for System, even when the OS is dark.
        var darkChoice = Choice(design.T("appearance.dark_theme"), "dark_palette", [("charcoal", "Wisp Charcoal"), ("codex", "Codex"), ("github", "GitHub Dark"), ("catppuccin", "Catppuccin Mocha"), ("gruvbox", "Gruvbox")]);
        var lightChoice = Choice(design.T("appearance.light_theme"), "light_palette", [("paper", "Wisp Paper"), ("codex", "Codex"), ("github", "GitHub"), ("catppuccin", "Catppuccin Latte"), ("everforest", "Everforest")]);
        showTheme = mode =>
        {
            foreach (var update in themeCards) update(mode);
            darkChoice.Visibility = mode == "dark" ? Visibility.Visible : Visibility.Collapsed;
            lightChoice.Visibility = mode == "dark" ? Visibility.Collapsed : Visibility.Visible;
        };
        showTheme(prefs["theme"]?.GetValue<string>() ?? "system");
        foreach (var (token, label) in new[] { ("clay", design.T("appearance.accent")), ("bg-app", design.T("appearance.background")), ("text", design.T("appearance.foreground")) })
        {
            var dot = new Border { Width = 16, Height = 16, CornerRadius = new CornerRadius(8), BorderThickness = new Thickness(1) };
            var value = design.Text("", 12); value.FontFamily = design.CodeFont; value.VerticalAlignment = VerticalAlignment.Center;
            var content = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 10 }; content.Children.Add(dot); content.Children.Add(value);
            var swatch = new Border { Child = content, Width = 150, Height = 32, Padding = new Thickness(10, 0, 10, 0), CornerRadius = new CornerRadius(8), HorizontalAlignment = HorizontalAlignment.Right };
            colorSamples[token] = (swatch, value, dot);
            AddRow(label, null, swatch, divider: token != "text");
        }
        target = new StackPanel(); var fontHeading = Heading(design.T("appearance.fonts"), 15); fontHeading.Margin = new Thickness(0, 0, 0, 16); target.Children.Add(fontHeading);
        var fontCard = design.Card(target); SizeAppearanceCard(fontCard); form.Children.Add(fontCard);
        FontSize("ui_font_size");
        FontFamilyField("ui_font_family", "Inter");
        FontSize("code_font_size");
        FontFamilyField("code_font_family", "JetBrains Mono");
        RenderCustomTheme(prefs);

        UpdatePreview(prefs);
        design.ApplyTypography(shell, previewCard);
        PreferenceFieldAppearance.ApplyTree(form, design);
        formHost.IsEnabled = loaded && !working;
        customTheme.IsEnabled = loaded && !working;

        Grid Choice(string label, string key, (string Value, string Label)[] items)
        {
            var choice = new ComboBox { HorizontalAlignment = HorizontalAlignment.Stretch, CornerRadius = new CornerRadius(8), Background = design.Brush("bg-input"),
                BorderBrush = design.Brush("border-strong"), Padding = new Thickness(11, 9, 0, 9) };
            AutomationProperties.SetName(choice, design.T("appearance.palette"));
            foreach (var item in items) choice.Items.Add(new ComboBoxItem { Content = item.Label, Tag = item.Value });
            choice.SelectedItem = choice.Items.Cast<ComboBoxItem>().FirstOrDefault(i => (string)i.Tag == prefs[key]?.GetValue<string>());
            choice.SelectionChanged += (_, _) => { if (choice.SelectedItem is ComboBoxItem item) ChangeAppearance(key, JsonValue.Create((string)item.Tag)); };
            choices.Add(choice); return AddRow(label, null, choice, .55);
        }
        void FontSize(string key)
        {
            var row = new Grid { ColumnSpacing = 8 };
            row.ColumnDefinitions.Add(new() { Width = new GridLength(1, GridUnitType.Star) }); row.ColumnDefinitions.Add(new() { Width = GridLength.Auto, MinWidth = 44 });
            var fonts = NativeTypography.From(prefs);
            var slider = new Slider { Minimum = 0, Maximum = 30, StepFrequency = 1, Value = key == "ui_font_size" ? fonts.UiSize : fonts.CodeSize, VerticalAlignment = VerticalAlignment.Center, MinWidth = 0 };
            PreferenceSliderAppearance.Apply(slider, design);
            AutomationProperties.SetName(slider, design.T("appearance." + key));
            var value = design.Text($"{slider.Value:0} px", 12.5); value.Foreground = design.Brush("text-muted"); value.TextWrapping = TextWrapping.NoWrap; value.VerticalAlignment = VerticalAlignment.Center; value.HorizontalAlignment = HorizontalAlignment.Right;
            slider.ValueChanged += (_, e) => { value.Text = $"{e.NewValue:0} px"; ChangeAppearance(key, JsonValue.Create((int)e.NewValue)); };
            row.Children.Add(slider); Grid.SetColumn(value, 1); row.Children.Add(value); AddRow(design.T("appearance." + key), design.T("appearance." + key + "_hint"), row);
        }
        void FontFamilyField(string key, string placeholder)
        {
            var input = new TextBox { Text = prefs[key]?.GetValue<string>() ?? "", PlaceholderText = placeholder, CornerRadius = new CornerRadius(8), Background = design.Brush("bg-input"),
                BorderBrush = design.Brush("border-strong"), BorderThickness = new Thickness(1), Padding = new Thickness(11, 9, 11, 9) };
            foreach (var state in new[] { "", "PointerOver", "Focused" })
            {
                input.Resources["TextControlBackground" + state] = design.Brush("bg-input");
                input.Resources["TextControlBorderBrush" + state] = design.Brush(state == "Focused" ? "clay" : "border-strong");
            }
            AutomationProperties.SetName(input, design.T("appearance." + key));
            PreferenceFieldAppearance.StylePlaceholder(input);
            textMenus.Register(input);
            input.TextChanged += (_, _) => ChangeAppearance(key, JsonValue.Create(input.Text));
            AddRow(design.T("appearance." + key), design.T("appearance." + key + "_hint"), input, divider: key != "code_font_family");
        }
        Grid AddRow(string label, string? hint, FrameworkElement control, double fraction = .48, bool divider = true)
        {
            var row = new Grid { ColumnSpacing = 14, MinHeight = 54, Padding = new Thickness(0, 10, 0, 10),
                BorderBrush = design.Brush("border"), BorderThickness = new Thickness(0, 0, 0, divider ? 1 : 0) };
            row.ColumnDefinitions.Add(new() { Width = new GridLength(1 - fraction, GridUnitType.Star) });
            row.ColumnDefinitions.Add(new() { Width = new GridLength(fraction, GridUnitType.Star) });
            row.RowDefinitions.Add(new() { Height = GridLength.Auto }); row.RowDefinitions.Add(new() { Height = GridLength.Auto });
            var text = new StackPanel { Spacing = 3, VerticalAlignment = VerticalAlignment.Center };
            var title = design.Text(label, 13); design.BindLineHeight(title, 13, 1.2); title.FontWeight = Microsoft.UI.Text.FontWeights.SemiBold; text.Children.Add(title);
            if (hint != null) { var detail = design.Text(hint, 11); detail.Foreground = design.Brush("text-faint"); design.BindLineHeight(detail, 11, 1.5); text.Children.Add(detail); }
            control.VerticalAlignment = VerticalAlignment.Center; row.Children.Add(text); row.Children.Add(control); Grid.SetColumn(control, 1);
            row.SizeChanged += (_, e) =>
            {
                var stacked = NativeSettingsLayout.AppearanceFieldsStack(e.NewSize.Width);
                row.ColumnSpacing = stacked ? 0 : 14;
                row.RowSpacing = stacked ? 8 : 0;
                row.ColumnDefinitions[1].Width = new GridLength(NativeSettingsLayout.AppearanceControlWidth(e.NewSize.Width, fraction));
                Grid.SetRow(control, stacked ? 1 : 0); Grid.SetColumn(control, stacked ? 0 : 1);
            };
            target.Children.Add(row); return row;
        }
    }
    private void UpdatePreview(JsonObject prefs)
    {
        var typography = NativeTypography.From(prefs);
        previewDesign.Dark = prefs["theme"]?.GetValue<string>() == "dark" ||
            (prefs["theme"]?.GetValue<string>() == "system" && shell.ActualTheme == ElementTheme.Dark);
        previewDesign.LightPalette = prefs["light_palette"]?.GetValue<string>() ?? "paper";
        previewDesign.DarkPalette = prefs["dark_palette"]?.GetValue<string>() ?? "charcoal";
        previewDesign.RefreshPalette();
        var palette = new WispDesign { Dark = prefs["theme"]?.GetValue<string>() == "dark", LightPalette = previewDesign.LightPalette, DarkPalette = previewDesign.DarkPalette };
        foreach (var (token, sample) in colorSamples)
        {
            var ink = token == "clay" ? palette.Dark && palette.DarkPalette == "gruvbox" ? "#1D2021" : "#FFFFFF"
                : token == "bg-app" ? palette.Dark ? "#FFFFFF" : "#1F2328" : palette.Dark ? "#1F2328" : "#FFFFFF";
            sample.Swatch.Background = palette.Brush(token); sample.Value.Text = palette.ColorText(token).ToUpperInvariant();
            sample.Value.Foreground = Solid(ink); sample.Dot.BorderBrush = Solid(ink, .3);
        }
        previewCard.Background = previewDesign.Brush("bg-elev"); previewCard.BorderBrush = previewDesign.Brush("border");
        preview.Children.Clear();
        var title = Text(design.T("appearance.live_preview"), 15, leading: 1.2); title.FontWeight = Microsoft.UI.Text.FontWeights.SemiBold; preview.Children.Add(title);
        var hint = Text(design.T("appearance.instant"), 12, "text-faint", leading: 1.45); hint.Margin = new Thickness(0, 16, 0, 24); preview.Children.Add(hint);
        preview.Children.Add(new Border { Child = Text(design.T("appearance.preview_question"), 14), Background = previewDesign.Brush("bg-sunken"),
            Padding = new Thickness(16, 12, 16, 12), CornerRadius = new CornerRadius(12), Margin = new Thickness(24, 0, 0, 24) });
        var answerTitle = Text(design.T("appearance.preview_heading"), 14); answerTitle.FontWeight = Microsoft.UI.Text.FontWeights.SemiBold; preview.Children.Add(answerTitle);
        var answer = Text(design.T("appearance.preview_answer"), 14); answer.Margin = new Thickness(0, 8, 0, 16); preview.Children.Add(answer);
        preview.Children.Add(new Border { Child = Text("summary(data)", 12, code: true), Background = previewDesign.Brush("bg-sunken"),
            BorderBrush = previewDesign.Brush("border"), BorderThickness = new Thickness(1), Padding = new Thickness(12), CornerRadius = new CornerRadius(8) });
        var diffTitle = Text(design.T("appearance.preview_diff"), 13, leading: 1.2); diffTitle.FontWeight = Microsoft.UI.Text.FontWeights.SemiBold; diffTitle.Margin = new Thickness(0, 24, 0, 12); preview.Children.Add(diffTitle);
        diffPreview.Children.Clear(); diffPreview.RowDefinitions.Clear(); diffPreview.ColumnDefinitions.Clear();
        diffPreview.ColumnDefinitions.Add(new() { Width = new GridLength(1, GridUnitType.Star) }); diffPreview.ColumnDefinitions.Add(new() { Width = new GridLength(0) });
        diffPreview.RowDefinitions.Add(new() { Height = GridLength.Auto }); diffPreview.RowDefinitions.Add(new() { Height = GridLength.Auto });
        foreach (var added in new[] { false, true })
        {
            var color = previewDesign.Brush(added ? "ok" : "err");
            var lines = new StackPanel { Background = previewDesign.Brush("bg-sunken") };
            for (var index = 0; index < 5; index++)
            {
                var row = new Grid { MinHeight = 26 };
                if (index is >= 1 and <= 3) row.Background = new SolidColorBrush(color.Color) { Opacity = .14 };
                row.ColumnDefinitions.Add(new() { Width = new GridLength(30) }); row.ColumnDefinitions.Add(new() { Width = new GridLength(1, GridUnitType.Star) });
                var number = Text((index + 1).ToString(), 12, "text-faint", true); number.HorizontalAlignment = HorizontalAlignment.Right; number.VerticalAlignment = VerticalAlignment.Center;
                row.Children.Add(new Border { Child = number, Padding = new Thickness(0, 0, 8, 0), BorderBrush = previewDesign.Brush("border"), BorderThickness = new Thickness(0, 0, 1, 0) });
                var code = Text("", 12, "text-muted", true); code.TextWrapping = TextWrapping.NoWrap; code.VerticalAlignment = VerticalAlignment.Center; code.Margin = new Thickness(12, 0, 0, 0);
                void Part(string text, Brush? ink = null, bool italic = false) => code.Inlines.Add(new Run { Text = text, Foreground = ink ?? previewDesign.Brush("text-muted"), FontStyle = italic ? Windows.UI.Text.FontStyle.Italic : Windows.UI.Text.FontStyle.Normal });
                if (index == 0) { Part("const", Solid("#7C3AED")); Part(" themePreview: "); Part("ThemeConfig", Solid("#D97706"), true); Part(" = {"); }
                else if (index == 1) { Part("  surface: "); Part(added ? "\"sidebar-elevated\"" : "\"sidebar\"", previewDesign.Brush("ok")); Part(","); }
                else if (index == 2) { Part("  accent: "); Part(added ? "\"#0ea5e9\"" : "\"#2563eb\"", previewDesign.Brush("ok")); Part(","); }
                else if (index == 3) { Part("  contrast: "); Part(added ? "68" : "42", Solid("#EA580C")); Part(","); }
                else Part("};");
                Grid.SetColumn(code, 1); row.Children.Add(code); lines.Children.Add(row);
            }
            // The reference uses an inset stripe inside the line-number gutter;
            // it doesn't consume four more pixels of each snippet's code width.
            var column = new Grid();
            column.Children.Add(lines);
            column.Children.Add(new Border { Width = 4, HorizontalAlignment = HorizontalAlignment.Left,
                Background = color, IsHitTestVisible = false });
            diffPreview.Children.Add(column);
        }
        LayoutDiffPreview();
        diffFrame.Background = previewDesign.Brush("bg-sunken"); diffFrame.BorderBrush = previewDesign.Brush("border");
        preview.Children.Add(diffFrame);
        LayoutDiffPreview();
        TextBlock Text(string text, double size, string token = "text", bool code = false, double leading = 1.65) => new() {
            Text = text, FontSize = typography.Scale(size, code), LineHeight = typography.Scale(size, code) * leading, LineStackingStrategy = LineStackingStrategy.MaxHeight,
            Foreground = previewDesign.Brush(token), TextWrapping = TextWrapping.Wrap, FontFamily = new FontFamily(typography.FontSource(code)) };
    }

    private void LayoutDiffPreview()
    {
        if (diffPreview.Children.Count != 2) return;
        var sideBySide = NativeSettingsLayout.AppearanceDiffColumns(shell.ActualWidth);
        diffPreview.ColumnDefinitions[1].Width = sideBySide ? new GridLength(1, GridUnitType.Star) : new GridLength(0);
        var added = (FrameworkElement)diffPreview.Children[1];
        Grid.SetColumn(added, sideBySide ? 1 : 0); Grid.SetRow(added, sideBySide ? 0 : 1);
    }

    private TextBlock NavigationText(string text, double size)
    {
        var block = design.Text(text, size); design.BindLineHeight(block, size, 1.2); return block;
    }
    private void SizeAppearanceCard(Border card)
    {
        void Resize()
        {
            var (horizontal, vertical) = NativeSettingsLayout.AppearanceCardPadding(shell.ActualWidth);
            card.Padding = new Thickness(horizontal, vertical, horizontal, vertical);
        }
        card.Loaded += (_, _) => Resize(); card.SizeChanged += (_, _) => Resize(); Resize();
    }
    private TextBlock Heading(string text, double size = 14)
    {
        var heading = design.Text(text, size); heading.FontWeight = Microsoft.UI.Text.FontWeights.SemiBold; design.BindLineHeight(heading, size, 1.2); return heading;
    }

    private static SolidColorBrush Solid(string hex, double opacity = 1) => new(Windows.UI.Color.FromArgb(255,
        Convert.ToByte(hex.Substring(1, 2), 16), Convert.ToByte(hex.Substring(3, 2), 16), Convert.ToByte(hex.Substring(5, 2), 16))) { Opacity = opacity };

    private Border ThemeSample(string mode)
    {
        Brush Split(string light, string dark, double offset)
        {
            var brush = new LinearGradientBrush { StartPoint = new(0, 0), EndPoint = new(1, 0) };
            brush.GradientStops.Add(new() { Color = Solid(light).Color, Offset = 0 }); brush.GradientStops.Add(new() { Color = Solid(light).Color, Offset = offset });
            brush.GradientStops.Add(new() { Color = Solid(dark).Color, Offset = offset }); brush.GradientStops.Add(new() { Color = Solid(dark).Color, Offset = 1 }); return brush;
        }
        var canvas = new Grid();
        var sample = new Border { Child = canvas, Height = 74, CornerRadius = new CornerRadius(8), BorderThickness = new Thickness(1),
            Background = mode == "system" ? Split("#F3F4F6", "#343536", .5) : Solid(mode == "dark" ? "#343536" : "#F3F4F6") };
        var window = new Grid { Background = Solid("#FFFFFF"), CornerRadius = new CornerRadius(10, 10, 0, 0) };
        window.ColumnDefinitions.Add(new() { Width = new GridLength(.28, GridUnitType.Star) }); window.ColumnDefinitions.Add(new() { Width = new GridLength(.72, GridUnitType.Star) });
        window.Children.Add(new Border { Background = mode == "system" ? Split("#E4E5E7", "#56585B", .47) : Solid(mode == "dark" ? "#56585B" : "#E4E5E7") });
        var lines = new StackPanel { VerticalAlignment = VerticalAlignment.Top };
        foreach (var width in new[] { .78, .58, .78 })
        {
            var line = new Border { Height = 3, Background = Solid("#D4D5D7"), CornerRadius = new CornerRadius(2), HorizontalAlignment = HorizontalAlignment.Left };
            lines.SizeChanged += (_, e) => line.Width = Math.Max(0, e.NewSize.Width * width); lines.Children.Add(line);
        }
        Grid.SetColumn(lines, 1); window.Children.Add(lines); canvas.Children.Add(window);
        window.SizeChanged += (_, e) =>
        {
            var ink = NativeSettingsLayout.ThemePreviewLines(e.NewSize.Width * .72, e.NewSize.Height);
            lines.Margin = new Thickness(ink.InsetX, ink.InsetY, ink.InsetX, 0); lines.Spacing = ink.Gap;
            foreach (var line in lines.Children.OfType<Border>()) line.Height = ink.Stroke;
        };
        sample.SizeChanged += (_, e) => { sample.Height = e.NewSize.Width / 2; window.Margin = new Thickness(e.NewSize.Width * .05, e.NewSize.Width * .12, e.NewSize.Width * .05, 0); };
        return sample;
    }

}
