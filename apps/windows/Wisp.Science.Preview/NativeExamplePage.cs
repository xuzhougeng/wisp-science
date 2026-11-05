using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Controls.Primitives;
using Wisp.ProjectBrowser;
using Windows.ApplicationModel.DataTransfer;

namespace Wisp.Science.Preview;

internal sealed class NativeExamplePage : UserControl, IDisposable
{
    private readonly WispDesign design;
    private readonly Grid root = new(), main = new();
    private readonly Border sidebar;
    private readonly Button expand, tab;
    private readonly TextBlock status;
    private readonly WorkspaceConversationModel model = WorkspaceConversationModel.CreateExampleViewer();
    private readonly NativeConversationPage transcript;
    private readonly NativeGeneratedArtifacts outputs = new();
    private readonly NativeArtifactsPane outputsPane;
    private readonly Border outputHost = new() { Visibility = Visibility.Collapsed };
    private Button? outputToggle;
    private readonly Dictionary<string, Button> sessions = [];
    private readonly List<FlyoutBase> menus = [];
    private bool collapsed, drawer, disposed, outputOpen;
    public NativeExamplePage(WispDesign design, Action back, Action<NativeExampleInfo> copy)
    {
        this.design = design;
        outputsPane = new(design);
        AutomationProperties.SetAutomationId(this, "example-project");
        root.ColumnDefinitions.Add(new() { Width = new GridLength(248) }); root.ColumnDefinitions.Add(new()); Content = root;
        var side = new Grid { Padding = new Thickness(10, 12, 10, 0) };
        side.RowDefinitions.Add(new() { Height = GridLength.Auto }); side.RowDefinitions.Add(new());
        var head = new Grid { Margin = new Thickness(0, 0, 0, 12), ColumnSpacing = 5 };
        head.ColumnDefinitions.Add(new() { Width = GridLength.Auto }); head.ColumnDefinitions.Add(new()); head.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
        var goBack = Tool(design.T("sidebar.back_projects"), "arrow-left", back); head.Children.Add(goBack);
        var title = design.Text(design.T("projects.example"), 14); title.FontWeight = Microsoft.UI.Text.FontWeights.SemiBold;
        title.TextWrapping = TextWrapping.NoWrap; title.TextTrimming = TextTrimming.CharacterEllipsis; title.VerticalAlignment = VerticalAlignment.Center;
        Grid.SetColumn(title, 1); head.Children.Add(title);
        var collapse = Tool(design.T("sidebar.collapse"), "chevron-left", () => { collapsed = true; drawer = false; Layout(); }); Grid.SetColumn(collapse, 2); head.Children.Add(collapse); side.Children.Add(head);
        var rows = new StackPanel { Spacing = 2 };
        foreach (var info in NativeExampleCatalog.Entries)
        {
            var row = new Grid(); row.ColumnDefinitions.Add(new()); row.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
            var label = design.Text(info.Title, 13); label.TextWrapping = TextWrapping.NoWrap; label.TextTrimming = TextTrimming.CharacterEllipsis;
            var open = new Button { Content = label, HorizontalAlignment = HorizontalAlignment.Stretch, HorizontalContentAlignment = HorizontalAlignment.Left, Padding = new Thickness(18, 6, 4, 6), MinHeight = 32, CornerRadius = new(8) };
            design.QuietButton(open); ToolTipService.SetToolTip(open, info.Title); AutomationProperties.SetName(open, info.Title); AutomationProperties.SetAutomationId(open, "demo-" + info.Id);
            open.Click += (_, _) => Open(info); row.Children.Add(open); sessions[info.Id] = open;
            var actions = Tool(design.T("demo.actions"), "more", () => { }); actions.Width = 28; Grid.SetColumn(actions, 1); row.Children.Add(actions);
            var menu = new MenuFlyout();
            var copyTitle = new MenuFlyoutItem { Text = design.L("复制标题", "Copy title") };
            copyTitle.Click += (_, _) => { try { var data = new DataPackage(); data.SetText(info.Title); Clipboard.SetContent(data); } catch { } };
            var copyProject = new MenuFlyoutItem { Text = design.L("复制到项目…", "Copy to a project…") }; copyProject.Click += (_, _) => copy(info);
            menu.Items.Add(copyTitle); menu.Items.Add(copyProject); menu.Opening += (_, _) => menus.Add(menu); menu.Closed += (_, _) => menus.Remove(menu);
            actions.Flyout = menu; open.ContextFlyout = menu; rows.Children.Add(row);
        }
        var list = new ScrollViewer { Content = rows, HorizontalScrollBarVisibility = ScrollBarVisibility.Disabled, HorizontalContentAlignment = HorizontalAlignment.Stretch };
        Grid.SetRow(list, 1); side.Children.Add(list);
        sidebar = new Border { Child = side, Background = design.Brush("bg-sunken"), BorderBrush = design.Brush("border"), BorderThickness = new(0, 0, 1, 0) }; root.Children.Add(sidebar);
        main.RowDefinitions.Add(new() { Height = GridLength.Auto }); main.RowDefinitions.Add(new()); main.RowDefinitions.Add(new() { Height = GridLength.Auto });
        main.ColumnDefinitions.Add(new()); main.ColumnDefinitions.Add(new() { Width = new GridLength(0) }); Grid.SetColumn(main, 1); root.Children.Add(main);
        var toolbar = new Grid { Padding = new(14, 9, 14, 10), ColumnSpacing = 8 };
        toolbar.ColumnDefinitions.Add(new() { Width = GridLength.Auto }); toolbar.ColumnDefinitions.Add(new() { Width = GridLength.Auto }); toolbar.ColumnDefinitions.Add(new()); toolbar.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
        expand = Tool(design.L("展开侧栏", "Show sidebar"), "chevron-right", () => { collapsed = false; drawer = ActualWidth < 680; Layout(); }); toolbar.Children.Add(expand);
        var tabLabel = design.Text(design.T("projects.example"), 12); tabLabel.TextWrapping = TextWrapping.NoWrap; tabLabel.TextTrimming = TextTrimming.CharacterEllipsis; tabLabel.TextAlignment = TextAlignment.Center;
        tab = new Button { Content = tabLabel, MinWidth = 0, Padding = new(14, 6, 14, 6), BorderBrush = design.Brush("border"), BorderThickness = new(1), Background = design.Brush("bg-input"), CornerRadius = new(7), HorizontalContentAlignment = HorizontalAlignment.Stretch };
        Grid.SetColumn(tab, 1); toolbar.Children.Add(tab);
        status = design.Text("", 11); status.Foreground = design.Brush("text-faint"); status.TextWrapping = TextWrapping.NoWrap; status.TextTrimming = TextTrimming.CharacterEllipsis; status.VerticalAlignment = VerticalAlignment.Center; Grid.SetColumn(status, 2); toolbar.Children.Add(status);
        var tools = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 1 };
        foreach (var (label, icon) in new[] { (design.T("share.topbar"), "share"), (design.T("trajectory.topbar"), "timeline"), (design.L("研究归档", "Archive research"), "archive"), (design.L("待查看", "Need you"), "bell"), (design.T("contexts.open_terminal"), "terminal"), (design.T("center.toggle_panel"), "panel") })
        {
            var button = Tool(label, icon, () => { }); button.IsEnabled = false; tools.Children.Add(button);
            if (icon == "panel") { outputToggle = button; button.IsEnabled = true; button.Click += (_, _) => { outputOpen = !outputOpen; if (!outputOpen) outputsPane.ClosePreview(); SyncOutputs(); Layout(); }; }
        }
        Grid.SetColumn(tools, 3); toolbar.Children.Add(tools);
        main.Children.Add(new Border { Child = toolbar, BorderBrush = design.Brush("border"), BorderThickness = new(0, 0, 0, 1) });
        Grid.SetColumnSpan((FrameworkElement)main.Children[0], 2);
        transcript = new(model, design, _ => { }, () => Task.CompletedTask, openGenerated: output => _ = OpenOutputAsync(output)); Grid.SetRow(transcript, 1); main.Children.Add(transcript);
        outputHost.BorderBrush = design.Brush("border"); outputHost.BorderThickness = new(1, 0, 0, 0); outputHost.Background = design.Brush("bg-app");
        var outputLayout = new Grid(); outputLayout.RowDefinitions.Add(new() { Height = GridLength.Auto }); outputLayout.RowDefinitions.Add(new());
        var outputHeader = new Grid { Padding = new(12, 8, 8, 8), ColumnSpacing = 8, Background = design.Brush("bg-sunken") };
        outputHeader.ColumnDefinitions.Add(new()); outputHeader.ColumnDefinitions.Add(new() { Width = GridLength.Auto }); outputHeader.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
        var outputTitle = design.Text(design.T("right.artifacts"), 12.5); outputTitle.FontWeight = Microsoft.UI.Text.FontWeights.SemiBold; outputTitle.VerticalAlignment = VerticalAlignment.Center;
        outputHeader.Children.Add(outputTitle); Grid.SetColumn(outputsPane.ViewModes, 1); outputHeader.Children.Add(outputsPane.ViewModes);
        var closeOutputs = Tool(design.T("right.close"), "close", CloseOutputs); Grid.SetColumn(closeOutputs, 2); outputHeader.Children.Add(closeOutputs);
        outputLayout.Children.Add(outputHeader); Grid.SetRow(outputsPane, 1); outputLayout.Children.Add(outputsPane); outputHost.Child = outputLayout;
        Grid.SetRow(outputHost, 1); Grid.SetColumn(outputHost, 1); Canvas.SetZIndex(outputHost, 1); main.Children.Add(outputHost);
        var noticeText = design.Text(design.T("projects.example_read_only"), 12.5); noticeText.TextAlignment = TextAlignment.Center; noticeText.Foreground = design.Brush("text-muted");
        var notice = new Border { Child = noticeText, Background = design.Brush("bg-input"), BorderBrush = design.Brush("border"), BorderThickness = new(1), CornerRadius = new(10), Padding = new(14, 11, 14, 11), Margin = new(16, 10, 16, 14), MaxWidth = PreviewLayout.ConversationMaxWidth, HorizontalAlignment = HorizontalAlignment.Stretch };
        AutomationProperties.SetAutomationId(notice, "demo-read-only"); Grid.SetRow(notice, 2); Grid.SetColumnSpan(notice, 2); main.Children.Add(notice);
        model.Changed += SyncOutputs;
        SizeChanged += (_, _) => Layout(); Layout();
    }
    private Button Tool(string label, string icon, Action action)
    { var button = design.ToolButton(label, icon); button.Width = button.Height = 32; button.Click += (_, _) => action(); return button; }
    private void Open(NativeExampleInfo info)
    {
        try
        {
            model.ShowExample(NativeExampleCatalog.Load(info.Id)); status.Text = design.Localization.Format("status.demo", ("title", info.Title));
            foreach (var (id, button) in sessions) button.Background = design.Brush(id == info.Id ? "bg-input" : "bg-sunken");
            if (ActualWidth < 680) { drawer = false; Layout(); }
        }
        catch (Exception ex) { status.Text = ex.Message; }
    }
    private void Layout()
    {
        var narrow = ActualWidth > 0 && ActualWidth < 680; var visible = !collapsed && (!narrow || drawer);
        root.ColumnDefinitions[0].Width = new GridLength(visible && !narrow ? 248 : 0);
        tab.Width = Math.Min(280, Math.Max(0, ActualWidth - root.ColumnDefinitions[0].Width.Value) * .36);
        sidebar.Visibility = visible ? Visibility.Visible : Visibility.Collapsed; sidebar.Width = 248;
        sidebar.HorizontalAlignment = HorizontalAlignment.Left; Grid.SetColumnSpan(sidebar, narrow ? 2 : 1); Canvas.SetZIndex(sidebar, 2);
        expand.Visibility = visible && !narrow ? Visibility.Collapsed : Visibility.Visible;
        var panelOpen = outputOpen; var panelOverlay = ActualWidth - root.ColumnDefinitions[0].Width.Value < 900;
        main.ColumnDefinitions[1].Width = new GridLength(panelOpen && !panelOverlay ? 400 : 0);
        Grid.SetColumn(outputHost, panelOverlay ? 0 : 1); Grid.SetColumnSpan(outputHost, panelOverlay ? 2 : 1);
        outputHost.HorizontalAlignment = panelOverlay ? HorizontalAlignment.Right : HorizontalAlignment.Stretch;
        outputHost.Width = panelOverlay ? Math.Min(400, Math.Max(0, ActualWidth - root.ColumnDefinitions[0].Width.Value)) : double.NaN;
        outputHost.Visibility = panelOpen ? Visibility.Visible : Visibility.Collapsed;
    }
    private void SyncOutputs() { outputs.Update(model.Snapshot, design.Localization); outputsPane.Update(outputs, [], ""); }
    private async Task OpenOutputAsync(NativeGeneratedOutput output) { SyncOutputs(); outputOpen = true; Layout(); await outputsPane.OpenAsync(output); }
    private void CloseOutputs() { outputOpen = false; outputsPane.ClosePreview(); Layout(); }
    public bool HandleEscape()
    {
        if (menus.LastOrDefault() is { } menu) { menu.Hide(); return true; }
        if (drawer) { drawer = false; Layout(); return true; }
        if (outputOpen && outputsPane.HandleEscape()) return true;
        if (outputOpen) { CloseOutputs(); return true; }
        if (transcript.HandleReadingEscape() || transcript.HandleHistoryEscape()) return true;
        return false;
    }
    public void Dispose() { if (disposed) return; disposed = true; foreach (var menu in menus.ToArray()) menu.Hide();
        model.Changed -= SyncOutputs; outputsPane.Dispose(); transcript.Dispose(); model.Pause(); }
}
