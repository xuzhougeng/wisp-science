using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Input;
using Microsoft.UI.Xaml.Media;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;
using Windows.Storage;
using Windows.Storage.Pickers;

namespace Wisp.Science.Preview;

internal interface IWorkspaceSheet : IDisposable
{
    bool IsModal => false;
    bool KeepsProjectSidebar => false;
    void HandleEscape();
}

internal abstract class WorkspaceSheet : UserControl, IWorkspaceSheet
{
    private readonly Action close;
    private bool closed;
    private bool settingsLayout;
    protected Action? BackToSettingsList { get; private set; }
    private double settingsContentMaxWidth = 1040;
    private Action? resizeSettings;
    private double? modalPreferredHeight;
    protected readonly WispDesign Design;
    private readonly Grid header;
    private readonly TextBlock heading;
    private readonly Button dismiss;
    private readonly Grid headingContent = new() { ColumnSpacing = 10 };
    public bool IsModal { get; }
    protected WorkspaceSheet(WispDesign design, string title, Action close, bool modal = false, double modalWidth = ProjectDialogLayout.PreferredWidth,
        bool scrollWholeModal = false, bool lightDismiss = false)
    {
        Design = design; this.close = close;
        IsModal = modal;
        HorizontalContentAlignment = HorizontalAlignment.Stretch;
        VerticalContentAlignment = VerticalAlignment.Stretch;
        design.BindTypography(this);
        var root = new Grid { Background = design.Brush("bg-app") };
        root.RowDefinitions.Add(new() { Height = GridLength.Auto });
        root.RowDefinitions.Add(new() { Height = GridLength.Auto });
        root.RowDefinitions.Add(new() { Height = new GridLength(1, GridUnitType.Star) });
        root.RowDefinitions.Add(new() { Height = GridLength.Auto });
        header = new Grid { Padding = new Thickness(28, 24, 28, 20), ColumnSpacing = 12, MaxWidth = 1040 };
        header.ColumnDefinitions.Add(new() { Width = new GridLength(1, GridUnitType.Star) });
        header.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
        header.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
        heading = design.Text(title, 24); heading.FontWeight = Microsoft.UI.Text.FontWeights.SemiBold;
        headingContent.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
        headingContent.ColumnDefinitions.Add(new() { Width = new GridLength(1, GridUnitType.Star) });
        Grid.SetColumn(heading, 1);
        heading.Foreground = design.Brush("text"); headingContent.Children.Add(heading); header.Children.Add(headingContent);
        dismiss = design.ToolButton(design.L("关闭", "Close ") + title, "close");
        dismiss.Click += (_, _) => HandleEscape();
        Grid.SetColumn(HeaderActions, 1); header.Children.Add(HeaderActions);
        Grid.SetColumn(dismiss, 2); header.Children.Add(dismiss);
        root.Children.Add(header);
        Notices = new StackPanel { Spacing = 8, Padding = new Thickness(20, 0, 20, 12), Visibility = Visibility.Collapsed };
        Grid.SetRow(Notices, 1); root.Children.Add(Notices);
        Body = new StackPanel { Spacing = 20, Padding = new Thickness(28, 0, 28, 28), MaxWidth = 1040 };
        BodyScroll = new ScrollViewer { Content = Body, HorizontalScrollBarVisibility = ScrollBarVisibility.Disabled,
            HorizontalScrollMode = ScrollMode.Disabled, HorizontalContentAlignment = HorizontalAlignment.Stretch };
        Grid.SetRow(BodyScroll, 2); root.Children.Add(BodyScroll);
        Footer = new StackPanel { Spacing = 8, Visibility = Visibility.Collapsed };
        Grid.SetRow(Footer, 3); root.Children.Add(Footer);
        Content = root;
        if (modal)
        {
            // Keep a finite width all the way through ScrollViewer and ContentControl.
            // Otherwise a long path or multiline editor can measure beyond the window.
            root.Background = design.Brush("bg-elev");
            root.CornerRadius = new CornerRadius(16);
            root.TabFocusNavigation = KeyboardNavigationMode.Cycle;
            header.MaxWidth = Body.MaxWidth = double.PositiveInfinity;
            headingContent.ColumnSpacing = 0;
            header.Padding = new Thickness(22, 22, 22, 14);
            Design.BindTypography(heading, 17);
            Body.Padding = new Thickness(22, 0, 22, 0); Body.Spacing = 14;
            Notices.Padding = new Thickness(22, 0, 22, 12);
            Footer.Padding = new Thickness(22, 18, 22, 22);
            if (scrollWholeModal)
            {
                // A WebView project form has one scroll surface, including the
                // title and actions. Keep the corner and backdrop stationary.
                root.Children.Remove(header); root.Children.Remove(Notices); root.Children.Remove(Footer);
                BodyScroll.Content = null;
                var form = new StackPanel();
                headingContent.VerticalAlignment = VerticalAlignment.Center;
                dismiss.MinHeight = dismiss.MinWidth = 0; dismiss.Height = dismiss.Width = 30; dismiss.Padding = new Thickness(0);
                form.Children.Add(header); form.Children.Add(Notices); form.Children.Add(Body); form.Children.Add(Footer);
                BodyScroll.Content = form;
                BodyScroll.VerticalScrollBarVisibility = ScrollBarVisibility.Auto;
                BodyScroll.RegisterPropertyChangedCallback(ScrollViewer.ScrollableHeightProperty,
                    (_, _) => form.Margin = new Thickness(0, 0, BodyScroll.ScrollableHeight > 0 ? ScrollGutter : 0, 0));
            }
            Content = null;
            var card = new Border { Child = root, Width = modalWidth,
                HorizontalAlignment = HorizontalAlignment.Center, VerticalAlignment = VerticalAlignment.Center,
                Background = design.Brush("bg-elev"), BorderBrush = design.Brush("border-strong"),
                BorderThickness = new Thickness(1), CornerRadius = new CornerRadius(16),
                Shadow = new ThemeShadow(), Translation = new System.Numerics.Vector3(0, 0, 24) };
            var overlay = new Grid();
            NativeDialogVisuals.Backdrop(overlay, design);
            if (lightDismiss) overlay.PointerPressed += (_, e) =>
            {
                if (ReferenceEquals(e.OriginalSource, overlay)) { e.Handled = true; HandleEscape(); }
            };
            // A transparent full-window owner intercepts pointer input; Escape is
            // still dispatched by MainWindow's stack, independent of input focus.
            overlay.Children.Add(card); Content = overlay;
            void Resize()
            {
                var layout = ModalBounds(ActualWidth, ActualHeight, modalWidth);
                card.Width = layout.Width; card.MaxHeight = Math.Min(layout.MaxHeight, ModalHeightLimit(ActualHeight));
                card.Height = modalPreferredHeight is { } preferred ? Math.Min(preferred, layout.MaxHeight) : double.NaN;
            }
            SizeChanged += (_, _) => Resize();
            Loaded += (_, _) => { Resize(); dismiss.Focus(FocusState.Programmatic); };
        }
        Loaded += (_, _) => Design.ApplyTypography(this);
    }
    protected StackPanel Body { get; }
    protected virtual double ScrollGutter => 10;
    protected virtual ProjectDialogLayout ModalBounds(double width, double height, double preferredWidth)
        => ProjectDialogLayout.ForViewport(width, height, preferredWidth);
    protected virtual double ModalHeightLimit(double viewportHeight) => double.PositiveInfinity;
    protected NativeActionWrap HeaderActions { get; } = new() { VerticalAlignment = VerticalAlignment.Center };
    protected ScrollViewer BodyScroll { get; }
    protected StackPanel Notices { get; }
    protected StackPanel Footer { get; }
    protected void HideDismissButton() => dismiss.Visibility = Visibility.Collapsed;
    protected void UseScrollingHeader(FrameworkElement content)
    {
        header.Visibility = Visibility.Collapsed;
        Body.Children.Insert(0, content);
    }
    protected void SetDismissEnabled(bool enabled) => dismiss.IsEnabled = enabled;
    protected void SetModalHeight(double height) => modalPreferredHeight = height;
    protected void SetModalHeading(string title, string subtitle)
    {
        heading.Text = title; heading.TextWrapping = TextWrapping.NoWrap; heading.TextTrimming = TextTrimming.CharacterEllipsis;
        ToolTipService.SetToolTip(heading, title); headingContent.Children.Clear();
        var copy = new StackPanel { Spacing = 3 }; copy.Children.Add(heading);
        var detail = Design.Text(subtitle, 11); detail.Foreground = Design.Brush("text-faint"); copy.Children.Add(detail);
        Grid.SetColumn(copy, 1); headingContent.Children.Add(copy);
        Microsoft.UI.Xaml.Automation.AutomationProperties.SetName(dismiss, Design.T("settings.cancel"));
    }
    protected void SettingsHeading(string title, Action? back = null)
    {
        BackToSettingsList = back;
        Microsoft.UI.Xaml.Automation.AutomationProperties.SetName(dismiss, Design.L("关闭", "Close ") + title);
        ToolTipService.SetToolTip(dismiss, Design.L("关闭", "Close ") + title);
        headingContent.Children.Clear();
        headingContent.ColumnSpacing = back == null ? 0 : 8;
        if (back != null)
        {
            var button = Design.ToolButton(Design.L("返回设置列表", "Back to settings"), "chevron-left");
            button.Click += (_, _) => back(); headingContent.Children.Add(button);
        }
        heading.Text = title; heading.VerticalAlignment = VerticalAlignment.Center;
        var margin = settingsLayout ? Design.FontSize(21 * .83) : 12;
        heading.Margin = back == null ? new Thickness(0, margin, 0, margin) : new Thickness(0);
        Design.BindTypography(heading, back == null ? settingsLayout ? 21 : 24 : 14); headingContent.Children.Add(heading);
        heading.FontWeight = settingsLayout && back == null ? Microsoft.UI.Text.FontWeights.Bold : Microsoft.UI.Text.FontWeights.SemiBold;
        header.Padding = back == null ? new Thickness(20, 16, 20, 14) : new Thickness(20, 12, 20, 12);
    }
    protected void SettingsBreadcrumb(string section, string detail, Action back)
    {
        SettingsHeading(detail, back);
        if (headingContent.Children[0] is Button backButton) StyleBreadcrumbBack(backButton);
        header.Padding = new Thickness(20, 16, 20, 14);
        headingContent.Children.Remove(heading);
        var trail = BreadcrumbTrail(section, detail, back); Grid.SetColumn(trail, 1); headingContent.Children.Add(trail);
    }
    protected void StyleBreadcrumbBack(Button backButton)
    {
        backButton.Width = backButton.Height = 30; backButton.MinWidth = backButton.MinHeight = 0;
        backButton.Padding = new Thickness(0); backButton.CornerRadius = new CornerRadius(15);
        backButton.BorderThickness = new Thickness(1); backButton.BorderBrush = Design.Brush("border-strong");
        backButton.Background = Design.Brush("bg-input");
        backButton.Content = Design.Icon("chevron-left", 18, Design.ColorText("text-muted"));
    }
    protected Grid BreadcrumbTrail(string section, string detail, Action back, int separatorSize = 12)
    {
        var crumbs = new Grid { ColumnSpacing = 8 };
        crumbs.ColumnDefinitions.Add(new() { Width = GridLength.Auto }); crumbs.ColumnDefinitions.Add(new() { Width = GridLength.Auto }); crumbs.ColumnDefinitions.Add(new() { Width = new GridLength(1, GridUnitType.Star) });
        var sectionButton = new Button { Content = Design.Text(section, 14) }; Design.QuietButton(sectionButton); sectionButton.Padding = new Thickness(0); sectionButton.MinHeight = 0;
        if (sectionButton.Content is TextBlock caption) caption.Foreground = Design.Brush("text-muted");
        sectionButton.Click += (_, _) => back(); crumbs.Children.Add(sectionButton);
        var separator = Design.Icon("chevron-right", separatorSize, Design.ColorText("text-faint")); Grid.SetColumn(separator, 1); crumbs.Children.Add(separator);
        var current = Design.Text(detail, 14); current.FontWeight = Microsoft.UI.Text.FontWeights.SemiBold; current.TextWrapping = TextWrapping.NoWrap; current.TextTrimming = TextTrimming.CharacterEllipsis;
        current.VerticalAlignment = VerticalAlignment.Center; Grid.SetColumn(current, 2); crumbs.Children.Add(current); return crumbs;
    }
    protected void UseSettingsLayout(bool compactPreferences = false, bool reserveScrollbar = false, bool reserveOverflowScrollbar = false)
    {
        Foreground = Design.Brush("text");
        settingsLayout = true; dismiss.Visibility = Visibility.Collapsed;
        header.MaxWidth = Body.MaxWidth = Notices.MaxWidth = Footer.MaxWidth = 1040;
        header.HorizontalAlignment = Body.HorizontalAlignment = Notices.HorizontalAlignment = Footer.HorizontalAlignment = HorizontalAlignment.Center;
        header.Padding = new Thickness(20, 16, 20, 14);
        heading.Margin = new Thickness(0, 12, 0, 12);
        header.BorderBrush = Design.Brush("border"); header.BorderThickness = new Thickness(0, 0, 0, 1);
        Body.Padding = new Thickness(24, 24, 24, 32); Body.Spacing = 20;
        Notices.Padding = new Thickness(24, 0, 24, 12);
        Footer.Padding = new Thickness(24, 12, 24, 20);
        void ResizeSettings()
        {
            // MaxWidth alone does not constrain a StackPanel measured by a ScrollViewer.
            var width = Math.Min(settingsContentMaxWidth, Math.Max(0, ActualWidth));
            header.Width = Body.Width = Notices.Width = Footer.Width = width;
            var gutter = reserveScrollbar || reserveOverflowScrollbar && BodyScroll.ScrollableHeight > 0;
            Body.Margin = new Thickness(0, 0, gutter ? 10 : 0, 0);
            if (gutter)
            {
                // WebView reserves its 10px scrollbar inside the centered pane.
                Body.Width = Math.Max(0, width - 10);
            }
            if (compactPreferences)
            {
                var compact = (XamlRoot?.Size.Width ?? ActualWidth) <= 850;
                Body.Padding = compact ? new Thickness(16) : new Thickness(24, 24, 24, 32);
                Body.Spacing = compact ? 16 : 20;
            }
        }
        SizeChanged += (_, _) => ResizeSettings();
        Loaded += (_, _) => ResizeSettings();
        if (reserveOverflowScrollbar)
        {
            // Network fits without a scrollbar on desktop; narrow windows need
            // the gutter when the same rows overflow the available height.
            Body.SizeChanged += (_, _) => ResizeSettings();
            BodyScroll.SizeChanged += (_, _) => ResizeSettings();
            BodyScroll.ViewChanged += (_, _) => ResizeSettings();
        }
        resizeSettings = ResizeSettings;
        Design.BindTypography(heading, 24);
    }
    protected void SettingsContentWidth(double width)
    {
        settingsContentMaxWidth = width;
        header.MaxWidth = Body.MaxWidth = Notices.MaxWidth = Footer.MaxWidth = width;
        resizeSettings?.Invoke();
    }
    public virtual void HandleEscape() { if (!closed) close(); }
    public virtual void Dispose() { closed = true; }
    protected TextBlock Mute(string text) { var block = Design.Text(text, 13); block.Foreground = Design.Brush("text-muted");
        block.Visibility = string.IsNullOrWhiteSpace(text) ? Visibility.Collapsed : Visibility.Visible; return block; }
    protected void PopulateLocalEnvironment(StackPanel page, NativeLocalEnvironmentModel env, double rowSpacing = 3)
    {
        string T(string key) => Design.T(key);
        TextBlock Hint(string message, double size)
        {
            var text = Design.Text(message, size, "text-muted"); text.LineHeight = Design.Typography.Scale(size) * 1.55; return text;
        }
        Button Action(string label, Action click)
        {
            var button = new Button { Content = label }; Design.ActionButton(button); button.Click += (_, _) => click(); return button;
        }
        page.Children.Clear(); page.Children.Add(Hint(T("env.optional"), 12));
        var list = new StackPanel { Spacing = rowSpacing, Margin = new Thickness(0, 4, 0, 4) };
        foreach (var (name, field) in NativeLocalEnvironmentModel.Fields)
        {
            var row = new Grid { ColumnSpacing = 8 }; row.ColumnDefinitions.Add(new() { Width = new GridLength(64) }); row.ColumnDefinitions.Add(new());
            var label = Design.Text(name, 12); label.FontWeight = Microsoft.UI.Text.FontWeights.SemiBold; row.Children.Add(label);
            FrameworkElement value;
            if (env.Draft is { } draft)
            {
                var input = new TextBox { Text = draft[field]?.ToString() ?? "", IsEnabled = !env.Busy, MinWidth = 0,
                    PlaceholderText = T("env.missing"), CornerRadius = new(8), Background = Design.Brush("bg-input"), BorderBrush = Design.Brush("border-strong") };
                Design.BindTypography(input, size: 12); Microsoft.UI.Xaml.Automation.AutomationProperties.SetName(input, name);
                input.TextChanged += (_, _) => draft[field] = input.Text; value = input;
            }
            else value = Design.Text(env.Paths?[field]?.ToString() is { Length: > 0 } path ? path
                : T(env.Paths == null && env.Error == null ? "env.checking" : "env.missing"), 12);
            Grid.SetColumn(value, 1); row.Children.Add(value); list.Children.Add(row);
        }
        page.Children.Add(list);
        if (env.Warning is { Length: > 0 } warning) page.Children.Add(Hint(warning, 12));
        if (env.Error != null) page.Children.Add(Warn(Design.L("无法确认本地环境路径。请重新检查。", "Could not confirm the local paths. Check them again.")));
        if (env.Saved) page.Children.Add(Hint(T("env.saved"), 12));
        if (env.Editing) page.Children.Add(Hint(T("env.edit_hint"), 12));
        var actions = new NativeActionWrap();
        actions.Children.Add(env.Editing ? Action(T("env.save"), () => _ = env.SaveAsync()) : Action(T("env.check_again"), () => _ = env.DetectAsync()));
        var edit = env.Editing ? Action(T("share.cancel"), env.Cancel) : Action(T("env.edit"), env.Edit);
        actions.Children.Add(edit);
        foreach (var button in actions.Children.OfType<Button>()) { button.IsEnabled = !env.Busy; button.MinHeight = 0; button.Padding = new Thickness(12, 5, 12, 5); Design.BindTypography(button, size: 12); }
        if (!env.Editing) edit.IsEnabled = env.CanEdit;
        page.Children.Add(actions);
    }
    protected TextBlock Warn(string text) { var block = Design.Text(text, 13); block.Foreground = Design.Brush("clay-strong"); return block; }
}

internal sealed class NativeOutlinePage : WorkspaceSheet
{
    private readonly WorkspaceOutlineModel model;
    private readonly CancellationTokenSource lifetime = new();
    private readonly TextBox search = new() { PlaceholderText = "搜索问题" };
    private readonly StackPanel list = new() { Spacing = 8 };
    private bool chrome;
    private readonly Func<ConversationOutlineEntry, CancellationToken, Task> navigate;
    private readonly Action close;
    private bool opening;
    private string? navigationError;
    public NativeOutlinePage(WorkspaceOutlineModel model, WispDesign design, Func<ConversationOutlineEntry, CancellationToken, Task> navigate, Action close) : base(design, "会话大纲", close)
    {
        this.model = model; this.navigate = navigate; this.close = close;
        search.TextChanged += (_, _) => { model.Query = search.Text; RenderList(); };
        _ = LoadAsync();
    }
    private async Task LoadAsync()
    {
        try { await model.RefreshAsync(lifetime.Token); Render(); }
        catch (OperationCanceledException) { }
    }
    private void Render()
    {
        if (!chrome)
        {
            chrome = true;
            var refresh = Design.ToolButton("刷新", "refresh");
            refresh.Click += async (_, _) => { await model.RefreshAsync(lifetime.Token); Render(); };
            HeaderActions.Children.Add(refresh); Body.Children.Add(search); Body.Children.Add(list);
        }
        RenderList();
    }
    private void RenderList()
    {
        list.Children.Clear();
        if (model.Loading) list.Children.Add(new ProgressBar { IsIndeterminate = true, Height = 3 });
        if (model.Error is { } error) list.Children.Add(Warn(error));
        if (navigationError is { } failed) list.Children.Add(Warn(failed));
        foreach (var entry in model.Visible)
        {
            var captured = entry;
            var row = new StackPanel { Spacing = 4 };
            row.Children.Add(new TextBlock { Text = $"{entry.UserIndex + 1}. {entry.Text}", TextWrapping = TextWrapping.Wrap });
            if (entry.SentAt is > 0 and var sent)
                row.Children.Add(Mute(entry.ResponseAt is { } response && response >= sent ? $"{response - sent} 秒" : ""));
            var button = new Button { Content = row, IsEnabled = !opening, HorizontalAlignment = HorizontalAlignment.Stretch, HorizontalContentAlignment = HorizontalAlignment.Left };
            Design.QuietButton(button); button.Padding = new Thickness(14); button.Background = Design.Brush("bg-elev");
            Microsoft.UI.Xaml.Automation.AutomationProperties.SetName(button, entry.Text);
            button.Click += async (_, _) =>
            {
                if (opening) return;
                opening = true; navigationError = null; RenderList();
                try
                {
                    await navigate(captured, lifetime.Token);
                    if (!lifetime.IsCancellationRequested) close();
                }
                catch (OperationCanceledException) { }
                catch (Exception ex) { navigationError = ex.Message; }
                finally { opening = false; if (!lifetime.IsCancellationRequested) RenderList(); }
            };
            list.Children.Add(button);
        }
        if (!model.Visible.Any() && !model.Loading && model.Error == null) list.Children.Add(Design.EmptyState("list", "暂无匹配的问题", "会话中的问题会列在这里，可搜索并返回对应位置。"));
    }
    public override void Dispose() { lifetime.Cancel(); model.Close(); lifetime.Dispose(); base.Dispose(); }
}

internal sealed class NativeInboxPage : WorkspaceSheet
{
    private readonly WorkspaceInboxModel model;
    private readonly string projectId;
    private readonly Func<NativeInboxEntry, Task> open;
    public NativeInboxPage(WorkspaceInboxModel model, string projectId, WispDesign design, Func<NativeInboxEntry, Task> open, Action close)
        : base(design, "待查看", close)
    {
        this.model = model; this.projectId = projectId; this.open = open;
        var refresh = Design.ToolButton("刷新", "refresh");
        refresh.Click += async (_, _) => { await model.RefreshAsync(projectId); RenderBody(); };
        HeaderActions.Children.Add(refresh);
        RenderBody();
    }
    public void RenderBody()
    {
        Body.Children.Clear();
        if (model.Error is { } error) Body.Children.Add(Warn(error));
        if (model.Loading) Body.Children.Add(new ProgressBar { IsIndeterminate = true, Height = 3 });
        foreach (var entry in model.Entries)
        {
            var captured = entry;
            var row = new StackPanel { Spacing = 4 };
            row.Children.Add(Mute(entry.ProjectName));
            row.Children.Add(new TextBlock { Text = entry.Title, TextWrapping = TextWrapping.Wrap });
            var button = new Button { Content = row, HorizontalAlignment = HorizontalAlignment.Stretch, HorizontalContentAlignment = HorizontalAlignment.Left };
            button.Click += async (_, _) => await open(captured);
            Body.Children.Add(button);
        }
        if (model.Entries.Length == 0 && !model.Loading && model.Error == null) Body.Children.Add(Design.EmptyState("list", "暂无待查看的会话", "需要你查看的会话会集中显示在这里。"));
    }
}

internal sealed class NativeTrajectoryPage : WorkspaceSheet
{
    private readonly WorkspaceTrajectoryModel model;
    private readonly Func<string, string, Task> saveHtml;
    private readonly TextBox search = new() { PlaceholderText = "搜索步骤、输入或输出" };
    private readonly ComboBox axis = new() { Header = "时间轴" };
    private readonly StackPanel list = new() { Spacing = 8 };
    private readonly TextBlock status = new() { TextWrapping = TextWrapping.Wrap };
    private bool chrome;
    public NativeTrajectoryPage(WorkspaceTrajectoryModel model, WispDesign design, Func<string, string, Task> saveHtml, Action close)
        : base(design, "运行轨迹", close)
    {
        this.model = model; this.saveHtml = saveHtml;
        foreach (var (label, value) in new (string, NativeTrajectoryAxis)[] { ("耗时", NativeTrajectoryAxis.Duration), ("轮次", NativeTrajectoryAxis.Turns), ("调用", NativeTrajectoryAxis.Calls) })
            axis.Items.Add(new ComboBoxItem { Content = label, Tag = value });
        axis.SelectedIndex = 0;
        search.TextChanged += (_, _) => { model.Query = search.Text; RenderList(); };
        axis.SelectionChanged += (_, _) =>
        {
            if (axis.SelectedItem is ComboBoxItem item) { model.Axis = (NativeTrajectoryAxis)item.Tag; RenderList(); }
        };
        _ = StartAsync();
    }
    private async Task StartAsync()
    {
        await model.RefreshAsync();
        Render();
    }
    private void Render()
    {
        if (!chrome)
        {
            chrome = true;
            var refresh = Design.ToolButton("刷新", "refresh");
            refresh.Click += async (_, _) => { await model.RefreshAsync(); Render(); };
            var export = new Button { Content = "导出 HTML" };
            Design.ActionButton(export);
            export.Click += async (_, _) =>
            {
                try { await saveHtml("wisp-trajectory.html", await model.ExportHtmlAsync()); }
                catch (Exception ex) { status.Text = ex.Message; status.Foreground = Design.Brush("clay-strong"); }
            };
            HeaderActions.Children.Add(refresh); HeaderActions.Children.Add(export);
            Body.Children.Add(search); Body.Children.Add(axis); Body.Children.Add(status); Body.Children.Add(list);
        }
        status.Text = model.Error ?? (model.Snapshot is { } snapshot
            ? $"{snapshot.Stats.Turns} 轮 · {snapshot.Stats.Steps} 步 · 模型 {snapshot.Stats.LlmMs} ms · 工具 {snapshot.Stats.ToolMs} ms"
            : "");
        status.Foreground = Design.Brush(model.Error == null ? "text-muted" : "clay-strong");
        RenderList();
    }
    private void RenderList()
    {
        list.Children.Clear();
        var chart = new Grid { Height = 36, ColumnSpacing = 1 };
        var column = 0;
        foreach (var segment in model.Segments)
        {
            chart.ColumnDefinitions.Add(new() { Width = new GridLength(Math.Max(0.5, segment.WidthPct), GridUnitType.Star) });
            var token = segment.Lane == "input" ? "traj-input-bar" : segment.Lane == "tools" ? "traj-tool-bar" : "traj-model-bar";
            var bar = new Border { Background = Design.Brush(token), CornerRadius = new CornerRadius(2) };
            Grid.SetColumn(bar, column++); chart.Children.Add(bar);
        }
        if (model.Segments.Length > 0) list.Children.Add(chart);
        foreach (var row in model.Rows)
        {
            var card = new StackPanel { Spacing = 4, Padding = new Thickness(10) };
            card.Children.Add(Mute(row.Cell.Kind + (row.Cell.DurationMs is { } ms ? $" · {ms} ms" : "")));
            card.Children.Add(new TextBlock { Text = row.Cell.Summary, TextWrapping = TextWrapping.Wrap });
            list.Children.Add(new Border { Child = card, Background = Design.Brush("bg-elev"), CornerRadius = new CornerRadius(8), Padding = new Thickness(4) });
        }
        if (model.Rows.Length == 0) list.Children.Add(Mute(model.Loading ? "正在读取轨迹…" : "没有匹配的轨迹记录"));
    }
    public override void Dispose() { model.Close(); base.Dispose(); }
}

internal sealed class NativeArchivePage : WorkspaceSheet
{
    private readonly WorkspaceArchiveModel model;
    private readonly Func<string, Task> continued;
    public NativeArchivePage(WorkspaceArchiveModel model, WispDesign design, Func<string, Task> continued, Action close)
        : base(design, "研究归档", close)
    {
        this.model = model; this.continued = continued;
        _ = StartAsync();
    }
    private async Task StartAsync() { await model.LoadAsync(); Render(); }
    public override void HandleEscape() { if (!model.Busy) base.HandleEscape(); }
    private void Render()
    {
        Body.Children.Clear();
        if (model.Error is { } error) Body.Children.Add(Warn(error));
        if (model.Busy) Body.Children.Add(Mute("正在整理或保存研究材料…"));
        if (model.Archive is { } archive)
        {
            Body.Children.Add(new TextBlock { Text = model.Frozen ? "已归档研究节点" : "确认研究归档", FontSize = 16 });
            var title = new TextBox { Header = "节点标题", Text = archive.Title, IsReadOnly = model.Frozen || model.Busy };
            var report = new TextBox { Header = "研究问题、结论、局限与选择依据", AcceptsReturn = true, Text = archive.Report, TextWrapping = TextWrapping.Wrap, MinHeight = 160, IsReadOnly = model.Frozen || model.Busy };
            Body.Children.Add(title); Body.Children.Add(report);
            foreach (var file in archive.Files)
                Body.Children.Add(Mute($"{file.Path} · {file.Action} · {file.SizeBytes} bytes"));
            Body.Children.Add(Mute(model.DeletedBytes > 0 ? $"将永久删除：{model.DeletedBytes} bytes" : "没有待删除文件"));
            foreach (var warning in archive.Warnings) Body.Children.Add(Warn(warning));
            if (model.Frozen)
            {
                var retry = new Button { Content = "重试未完成清理", IsEnabled = !model.Busy };
                retry.Click += async (_, _) => { await model.RetryCleanupAsync(); Render(); };
                var next = new Button { Content = "继续研究", IsEnabled = !model.Busy };
                next.Click += async (_, _) => { if (await model.ContinueAsync() is { } id) await continued(id); else Render(); };
                Body.Children.Add(retry); Body.Children.Add(next);
            }
            else
            {
                var consent = new CheckBox { Content = "我已检查归档材料。确认后会话将只读，所选文件立即永久删除，无法撤销。", IsChecked = model.Accepted, IsEnabled = !model.Busy };
                consent.Checked += (_, _) => model.Accepted = true;
                consent.Unchecked += (_, _) => model.Accepted = false;
                var prepare = new Button { Content = "重新整理", IsEnabled = !model.Busy };
                prepare.Click += async (_, _) => { await model.PrepareAsync(); Render(); };
                var confirm = new Button { Content = "确认归档并清理", IsEnabled = model.CanConfirm };
                confirm.Click += async (_, _) =>
                {
                    model.Edit(title.Text, report.Text);
                    await model.ConfirmAsync(); Render();
                };
                Body.Children.Add(consent); Body.Children.Add(prepare); Body.Children.Add(confirm);
            }
        }
        else if (!model.Busy)
        {
            var retry = new Button { Content = "重新读取" };
            retry.Click += async (_, _) => { await model.LoadAsync(); Render(); };
            Body.Children.Add(retry);
        }
    }
}

internal static class NativeWorkspaceFiles
{
    public static async Task<string?> PickDownloadAsync(Window window, string name, string fileLabel)
    {
        var safeName = NativeFileListPresentation.DownloadName(name);
        var picker = new FileSavePicker { SuggestedFileName = safeName };
        var extension = System.IO.Path.GetExtension(safeName);
        picker.FileTypeChoices.Add(fileLabel, [extension.Length > 1 ? extension : ".download"]);
        WinRT.Interop.InitializeWithWindow.Initialize(picker, WinRT.Interop.WindowNative.GetWindowHandle(window));
        return (await picker.PickSaveFileAsync())?.Path;
    }
    public static async Task<string?> SaveShareAsync(Window window, NativeShareFile content)
    {
        var picker = new FileSavePicker();
        WinRT.Interop.InitializeWithWindow.Initialize(picker, WinRT.Interop.WindowNative.GetWindowHandle(window));
        picker.FileTypeChoices.Add(content.Extension == ".png" ? "PNG" : "HTML", [content.Extension]);
        picker.SuggestedFileName = content.Name;
        if (await picker.PickSaveFileAsync() is not { } file) return null;
        await FileIO.WriteBytesAsync(file, content.Contents);
        return file.Path;
    }
    public static async Task SaveHtmlAsync(Window window, string name, string contents)
    {
        var picker = new FileSavePicker();
        WinRT.Interop.InitializeWithWindow.Initialize(picker, WinRT.Interop.WindowNative.GetWindowHandle(window));
        picker.FileTypeChoices.Add("HTML", [".html"]);
        picker.SuggestedFileName = name;
        if (await picker.PickSaveFileAsync() is { } file)
            await FileIO.WriteTextAsync(file, contents);
    }
}
