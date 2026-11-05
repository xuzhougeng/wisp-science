using System.Text.Json.Nodes;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Controls.Primitives;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Data;
using Microsoft.UI.Xaml.Input;
using Microsoft.UI.Xaml.Media;
using Microsoft.UI.Xaml.Shapes;
using Windows.Foundation;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

namespace Wisp.Science.Preview;

/// Native template library, DAG canvas and inspector, with one window-owned Escape path.
internal sealed partial class NativeWorkflowStudio : UserControl, IDisposable
{
    private readonly NativeWorkflowStudioModel model;
    private readonly INativeSettingsClient conversionClient;
    public NativeWorkflowConversionModel ConversionState { get; init; }
    public bool OpenConversionInitially { get; init; }
    private readonly WispDesign design;
    private readonly Action back;
    private readonly Action conversion;
    private readonly Grid root = new(), layout = new(), editor = new(), workspace = new();
    private readonly ScrollViewer studioScroll = new() { HorizontalScrollBarVisibility = ScrollBarVisibility.Disabled, HorizontalContentAlignment = HorizontalAlignment.Stretch, VerticalContentAlignment = VerticalAlignment.Stretch };
    private readonly Border libraryFrame, inspectorFrame;
    private readonly Thumb libraryResizer, inspectorResizer;
    private readonly StackPanel library = new(), libraryHead = new(), inspector = new() { Spacing = 11, Padding = new Thickness(16, 4, 16, 16) };
    private readonly Grid identity = new() { ColumnSpacing = 6, RowSpacing = 3, MaxWidth = 640, HorizontalAlignment = HorizontalAlignment.Left };
    private readonly NativeActionWrap headerActions = new(), tools = new();
    private readonly TextBlock summary = new(), status = new() { TextWrapping = TextWrapping.Wrap };
    private readonly Canvas canvas = new(), canvasHost = new();
    private readonly Grid inspectorLayout = new(), configurationPane = new();
    private readonly ScrollViewer configurationScroll = new() { HorizontalScrollBarVisibility = ScrollBarVisibility.Disabled, HorizontalContentAlignment = HorizontalAlignment.Stretch };
    private readonly ScrollViewer selectedScroll = new() { HorizontalScrollBarVisibility = ScrollBarVisibility.Disabled, HorizontalContentAlignment = HorizontalAlignment.Stretch };
    private readonly StackPanel selectedSummary = new() { Padding = new Thickness(16) };
    private StackPanel? headerSummary;
    private readonly Dictionary<long, Button> nodeCards = [];
    private readonly Dictionary<long, Button> nodeRemoveButtons = [];
    private readonly HashSet<long> hoveredNodes = [];
    private TextBlock? policyText, policyHelp, taskCountText;
    private readonly List<(NativeWorkflowEdge Edge, Microsoft.UI.Xaml.Shapes.Path Path, Polygon Arrow)> edgePaths = [];
    private readonly Canvas minimap = new() { Width = 132, Height = 86, IsHitTestVisible = false };
    private readonly Border minimapFrame;
    private readonly Microsoft.UI.Xaml.Shapes.Path dots = new() { IsHitTestVisible = false, Opacity = .22 };
    private readonly ScrollViewer viewport = new() { HorizontalScrollBarVisibility = ScrollBarVisibility.Auto, VerticalScrollBarVisibility = ScrollBarVisibility.Auto, HorizontalScrollMode = ScrollMode.Enabled };
    private readonly ContentControl contentHost = new() { HorizontalContentAlignment = HorizontalAlignment.Stretch, VerticalContentAlignment = VerticalAlignment.Stretch };
    private readonly SettingsTextMenus menus = new();
    private readonly List<ComboBox> choices = [];
    private readonly List<Grid> dialogs = [];
    private NativeWorkflowGraph graph = NativeWorkflowGraph.Build([]);
    private Button? saveButton, zoomButton, conversionButton;
    private readonly NativeWorkflowConnection connection = new();
    private string? connecting => connection.Source;
    private Microsoft.UI.Xaml.Input.Pointer? connectionPointer;
    private readonly Microsoft.UI.Xaml.Shapes.Path connectionPreview = new() { IsHitTestVisible = false, StrokeThickness = 2, StrokeDashArray = new DoubleCollection { 5, 4 } };
    private bool suppressPortClick;
    private bool closed, rendering, autoFit = true, configurationExpanded = true, contextExpanded;
    private int zoom = 100;
    private double? libraryWidth, inspectorWidth;
    public bool Busy => model.Busy;

    public NativeWorkflowStudio(INativeSettingsClient client, string? projectId, WispDesign design, Action back, Action conversion, string? templateId = null)
    {
        this.design = design; this.back = back; this.conversion = conversion;
        conversionClient = client; ConversionState = new(projectId ?? "");
        model = new(client, projectId, design.Localization); model.Changed += UpdateState;
        HorizontalContentAlignment = HorizontalAlignment.Stretch; VerticalContentAlignment = VerticalAlignment.Stretch;
        design.BindTypography(this); root.Background = design.Brush("bg-app"); Content = root;
        layout.ColumnDefinitions.Add(new() { Width = new GridLength(240) }); layout.ColumnDefinitions.Add(new() { Width = new GridLength(1, GridUnitType.Star) });
        layout.RowDefinitions.Add(new() { Height = new GridLength(1, GridUnitType.Star) }); layout.RowDefinitions.Add(new() { Height = GridLength.Auto });
        studioScroll.Content = layout; contentHost.Content = studioScroll; root.Children.Add(contentHost);
        var libraryScroll = new ScrollViewer { Content = library, HorizontalScrollBarVisibility = ScrollBarVisibility.Disabled, HorizontalContentAlignment = HorizontalAlignment.Stretch };
        var libraryPane = new Grid(); libraryPane.RowDefinitions.Add(new() { Height = GridLength.Auto }); libraryPane.RowDefinitions.Add(new() { Height = new GridLength(1, GridUnitType.Star) });
        libraryPane.Children.Add(libraryHead); Grid.SetRow(libraryScroll, 1); libraryPane.Children.Add(libraryScroll);
        libraryFrame = new Border { Child = libraryPane, Background = design.Brush("bg-sunken"), BorderBrush = design.Brush("border"), BorderThickness = new Thickness(0, 0, 1, 0) }; layout.Children.Add(libraryFrame);
        editor.RowDefinitions.Add(new() { Height = GridLength.Auto }); editor.RowDefinitions.Add(new() { Height = GridLength.Auto }); editor.RowDefinitions.Add(new() { Height = new GridLength(1, GridUnitType.Star) });
        Grid.SetColumn(editor, 1); layout.Children.Add(editor);
        var header = new Grid { Padding = new Thickness(16, 10, 16, 10), MinHeight = 72, ColumnSpacing = 16, Background = design.Brush("bg-elev"), BorderBrush = design.Brush("border"), BorderThickness = new Thickness(0, 0, 0, 1) };
        header.ColumnDefinitions.Add(new() { Width = new GridLength(1, GridUnitType.Star) }); header.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
        header.RowDefinitions.Add(new() { Height = GridLength.Auto }); header.RowDefinitions.Add(new() { Height = GridLength.Auto });
        identity.ColumnDefinitions.Add(new() { Width = GridLength.Auto }); identity.ColumnDefinitions.Add(new() { Width = new GridLength(1, GridUnitType.Star) });
        identity.RowDefinitions.Add(new() { Height = GridLength.Auto }); identity.RowDefinitions.Add(new() { Height = GridLength.Auto });
        header.Children.Add(identity); Grid.SetColumn(headerActions, 1); headerActions.VerticalAlignment = VerticalAlignment.Center; header.Children.Add(headerActions); editor.Children.Add(header);
        header.SizeChanged += (_, e) =>
        {
            var narrow = NativeWorkflowLayout.StackPanels(ActualWidth) || e.NewSize.Width < 440;
            header.RowSpacing = narrow ? 8 : 0;
            Grid.SetRow(headerActions, narrow ? 1 : 0); Grid.SetColumn(headerActions, narrow ? 0 : 1);
            Grid.SetColumnSpan(headerActions, narrow ? 2 : 1); Grid.SetColumnSpan(identity, narrow ? 2 : 1);
            headerActions.HorizontalAlignment = HorizontalAlignment.Right;
        };
        status.Margin = new Thickness(18, 8, 18, 8); status.Foreground = design.Brush("clay-strong"); Grid.SetRow(status, 1); editor.Children.Add(status);
        workspace.ColumnDefinitions.Add(new() { Width = new GridLength(1, GridUnitType.Star) }); workspace.ColumnDefinitions.Add(new() { Width = new GridLength(270) });
        workspace.RowDefinitions.Add(new() { Height = new GridLength(1, GridUnitType.Star) }); workspace.RowDefinitions.Add(new() { Height = GridLength.Auto });
        Grid.SetRow(workspace, 2); editor.Children.Add(workspace);
        var graphArea = new Grid { Background = design.Brush("bg-sunken") }; graphArea.RowDefinitions.Add(new() { Height = GridLength.Auto }); graphArea.RowDefinitions.Add(new() { Height = new GridLength(1, GridUnitType.Star) });
        var toolbar = new Grid { Padding = new Thickness(12, 8, 12, 8), ColumnSpacing = 8, BorderBrush = design.Brush("border"), BorderThickness = new Thickness(0, 0, 0, 1) };
        toolbar.ColumnDefinitions.Add(new() { Width = new GridLength(1, GridUnitType.Star) }); toolbar.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
        toolbar.RowDefinitions.Add(new() { Height = GridLength.Auto }); toolbar.RowDefinitions.Add(new() { Height = GridLength.Auto });
        toolbar.SizeChanged += (_, e) =>
        {
            var stacked = NativeWorkflowLayout.StackToolbar(e.NewSize.Width); toolbar.RowSpacing = stacked ? 8 : 0;
            Grid.SetColumnSpan(summary, stacked ? 2 : 1); Grid.SetRow(tools, stacked ? 1 : 0); Grid.SetColumn(tools, stacked ? 0 : 1);
            Grid.SetColumnSpan(tools, stacked ? 2 : 1); tools.HorizontalAlignment = HorizontalAlignment.Right;
        };
        summary.TextWrapping = TextWrapping.Wrap; summary.VerticalAlignment = VerticalAlignment.Center; design.BindTypography(summary, 12); toolbar.Children.Add(summary);
        Grid.SetColumn(tools, 1); toolbar.Children.Add(tools); graphArea.Children.Add(toolbar);
        viewport.Content = canvasHost; dots.Fill = design.Brush("text-faint"); Grid.SetRow(dots, 1); graphArea.Children.Add(dots); canvasHost.Children.Add(canvas);
        Grid.SetRow(viewport, 1); graphArea.Children.Add(viewport); workspace.Children.Add(graphArea);
        minimapFrame = new Border { Child = minimap, Background = design.Brush("bg-elev"), BorderBrush = design.Brush("border-strong"), BorderThickness = new Thickness(1), CornerRadius = new CornerRadius(8), Padding = new Thickness(5), Margin = new Thickness(12), HorizontalAlignment = HorizontalAlignment.Right, VerticalAlignment = VerticalAlignment.Bottom, IsHitTestVisible = false };
        Grid.SetRow(minimapFrame, 1); graphArea.Children.Add(minimapFrame);
        inspectorLayout.RowDefinitions.Add(new() { Height = new GridLength(1, GridUnitType.Star) }); inspectorLayout.RowDefinitions.Add(new() { Height = GridLength.Auto });
        configurationPane.RowDefinitions.Add(new() { Height = GridLength.Auto }); configurationPane.RowDefinitions.Add(new() { Height = new GridLength(1, GridUnitType.Star) });
        var configurationHeading = new Grid { ColumnSpacing = 8 }; configurationHeading.ColumnDefinitions.Add(new() { Width = GridLength.Auto }); configurationHeading.ColumnDefinitions.Add(new() { Width = new GridLength(1, GridUnitType.Star) }); configurationHeading.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
        configurationHeading.Children.Add(new Border { Child = design.Icon("gear", 14, design.ColorText("text-muted")), Width = 26, Height = 26, CornerRadius = new CornerRadius(7), BorderThickness = new Thickness(1), BorderBrush = design.Brush("border"), Background = design.Brush("bg-sunken") });
        var configurationTitle = Text(T("workflow_studio.configuration"), 11.5, true); configurationTitle.VerticalAlignment = VerticalAlignment.Center; Grid.SetColumn(configurationTitle, 1); configurationHeading.Children.Add(configurationTitle);
        var configurationChevron = design.Icon("chevron-down", 14); Grid.SetColumn(configurationChevron, 2); configurationHeading.Children.Add(configurationChevron);
        var configurationButton = Button(T("workflow_studio.configuration"), () => { configurationExpanded = !configurationExpanded; configurationScroll.Visibility = configurationExpanded ? Visibility.Visible : Visibility.Collapsed; configurationChevron.Source = design.Icon(configurationExpanded ? "chevron-down" : "chevron-right", 14).Source; ResizeInspector(); });
        design.QuietButton(configurationButton); configurationButton.Content = configurationHeading; configurationButton.HorizontalContentAlignment = HorizontalAlignment.Stretch; configurationButton.HorizontalAlignment = HorizontalAlignment.Stretch; configurationButton.MinHeight = 44; configurationButton.Padding = new Thickness(16, 7, 16, 7); configurationPane.Children.Add(configurationButton);
        configurationPane.BorderBrush = design.Brush("border"); configurationPane.BorderThickness = new Thickness(0, 0, 0, 1);
        configurationScroll.Content = inspector; configurationScroll.BorderBrush = design.Brush("border"); configurationScroll.BorderThickness = new Thickness(0, 1, 0, 0); Grid.SetRow(configurationScroll, 1); configurationPane.Children.Add(configurationScroll); inspectorLayout.Children.Add(configurationPane);
        selectedScroll.Content = selectedSummary; var selectedFrame = new Border { Child = selectedScroll, BorderBrush = design.Brush("border-strong"), BorderThickness = new Thickness(0, 1, 0, 0), VerticalAlignment = VerticalAlignment.Bottom };
        Grid.SetRow(selectedFrame, 1); inspectorLayout.Children.Add(selectedFrame);
        inspectorFrame = new Border { Child = inspectorLayout, Background = design.Brush("bg-elev"), BorderBrush = design.Brush("border"), BorderThickness = new Thickness(1, 0, 0, 0) };
        Grid.SetColumn(inspectorFrame, 1); workspace.Children.Add(inspectorFrame);
        libraryResizer = Resizer(T("workflow_studio.resize_library"), false); layout.Children.Add(libraryResizer);
        inspectorResizer = Resizer(T("workflow_studio.resize_sidebar"), true); Grid.SetColumn(inspectorResizer, 1); workspace.Children.Add(inspectorResizer);
        connectionPreview.Stroke = design.Brush("clay");
        canvas.AddHandler(PointerMovedEvent, new PointerEventHandler(MoveConnection), true);
        canvas.AddHandler(PointerReleasedEvent, new PointerEventHandler(ReleaseConnection), true);
        canvas.PointerCanceled += (_, _) => CancelConnection();
        canvas.PointerCaptureLost += (_, _) => { if (connectionPointer != null) CancelConnection(); };
        canvasHost.PointerPressed += (_, e) => { if (connectionPointer == null && connecting != null) { CancelConnection(); e.Handled = true; } };
        SizeChanged += (_, _) => ResizeColumns();
        viewport.SizeChanged += (_, _) => { if (autoFit) zoom = graph.FitZoom(viewport.ActualWidth, viewport.ActualHeight); PositionCanvas(); };
        canvasHost.DoubleTapped += (_, e) => { if (ReferenceEquals(e.OriginalSource, canvasHost) && model.Draft != null) OpenTask(null, model.NewTask()); };
        Loaded += async (_, _) =>
        {
            try { if (await model.LoadAsync(templateId) && !closed) { ConversionState.Configure(model.Skills, model.Models); ConversionState.Changed += RefreshConversion; Render(); if (OpenConversionInitially) OpenConversion(); } }
            catch (Exception ex) { rendering = false; if (!closed) model.Fail(L("工作流界面加载失败：", "Could not load Workflow Studio: ") + ex.Message); }
        };
    }
    private string T(string key) => design.T(key);
    private string L(string zh, string en) => design.L(zh, en);
    private string F(string key, params (string Name, string Value)[] values) => design.Localization.Format(key, values);
    private string Role(JsonObject task) => S(task, "task_kind") == "run_activity" ? T("agents.run_activity")
        : S(task, "specialist_id") is { Length: > 0 } id ? S(model.Specialists.FirstOrDefault(row => S(row, "id") == id), "name") is { Length: > 0 } name ? name : id : T("agents.task.temporary");
    private static string S(JsonNode? value, string key) => NativeWorkflowStudioModel.Text(value, key);
    private TextBlock Text(string value, double size = 13, bool bold = false)
    { var text = design.Text(value, size); if (bold) text.FontWeight = Microsoft.UI.Text.FontWeights.SemiBold; return text; }
    private TextBlock Hint(string value) { var text = Text(value, 12); text.Foreground = design.Brush("text-muted"); return text; }
    private static TextBlock Leading(TextBlock text, double multiplier)
    {
        text.LineHeight = text.FontSize * multiplier; text.LineStackingStrategy = LineStackingStrategy.BlockLineHeight;
        text.RegisterPropertyChangedCallback(TextBlock.FontSizeProperty, (sender, _) => { var label = (TextBlock)sender; label.LineHeight = label.FontSize * multiplier; });
        return text;
    }
    private SolidColorBrush Mix(string foreground, string background, double weight)
    {
        var a = design.Brush(foreground).Color; var b = design.Brush(background).Color;
        byte Blend(byte x, byte y) => (byte)Math.Round(x * weight + y * (1 - weight));
        return new SolidColorBrush(Windows.UI.Color.FromArgb(Blend(a.A, b.A), Blend(a.R, b.R), Blend(a.G, b.G), Blend(a.B, b.B)));
    }
    private TextBlock MonoText(string value, double size, bool bold = false)
    {
        var text = Text(value, size, bold);
        text.SetBinding(TextBlock.FontFamilyProperty, new Binding { Source = design, Path = new PropertyPath(nameof(WispDesign.CodeFont)), Mode = BindingMode.OneWay });
        return text;
    }
    private Button Button(string title, Action action, bool primary = false)
    { var button = new Button { Content = title }; design.ActionButton(button, primary); AutomationProperties.SetName(button, title); button.Click += (_, _) => action(); return button; }
    private Button Icon(string title, string kind, Action action)
    { var button = design.ToolButton(title, kind); button.Click += (_, _) => action(); return button; }
    private TextBox Input(string label, string value, Action<string> changed, bool multiline = false)
    {
        var input = new TextBox { Header = label.Length == 0 ? null : label, AcceptsReturn = multiline, Text = value, TextWrapping = TextWrapping.Wrap,
            MinHeight = multiline ? 110 : 32, CornerRadius = new CornerRadius(8), Background = design.Brush("bg-input"), BorderBrush = design.Brush("border-strong"), BorderThickness = new Thickness(1) };
        design.BindTypography(input); AutomationProperties.SetName(input, label); menus.Register(input);
        input.TextChanged += (_, _) => { if (!rendering) changed(input.Text); }; return input;
    }
    private ComboBox Choice(string label, string value, IEnumerable<(string Id, string Label)> options, Action<string> changed)
    {
        var combo = new ComboBox { Header = label, HorizontalAlignment = HorizontalAlignment.Stretch, CornerRadius = new CornerRadius(8) };
        foreach (var (id, text) in options) combo.Items.Add(new ComboBoxItem { Tag = id, Content = text });
        combo.SelectedItem = combo.Items.Cast<ComboBoxItem>().FirstOrDefault(i => (string)i.Tag == value);
        combo.SelectionChanged += (_, _) => { if (!rendering && combo.SelectedItem is ComboBoxItem item) changed((string)item.Tag); };
        choices.Add(combo); design.BindTypography(combo); return combo;
    }
    private UIElement SharedContext(JsonObject proposal)
    {
        var group = new StackPanel { Spacing = 7 };
        var input = Input("", S(proposal, "context"), value => model.Change(proposal, "context", JsonValue.Create(value)), true);
        input.MinHeight = 140; input.PlaceholderText = T("agents.shared_context_ph"); design.BindTypography(input, size: 12); AutomationProperties.SetName(input, T("agents.shared_context"));
        var heading = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 4 };
        var arrow = design.Icon(contextExpanded ? "chevron-down" : "chevron-right", 10, design.ColorText("text-muted")); heading.Children.Add(arrow);
        var label = Text(T("agents.shared_context"), 11, true); label.Foreground = design.Brush("text-muted"); heading.Children.Add(label);
        var toggle = new ToggleButton { Content = heading, IsChecked = contextExpanded, Padding = new Thickness(0), MinHeight = 18, MinWidth = 0, BorderThickness = new Thickness(0), HorizontalAlignment = HorizontalAlignment.Left, Background = new SolidColorBrush(Microsoft.UI.Colors.Transparent) };
        foreach (var state in new[] { "", "PointerOver", "Pressed", "Checked", "CheckedPointerOver", "CheckedPressed" }) toggle.Resources["ToggleButtonBackground" + state] = toggle.Background;
        AutomationProperties.SetName(toggle, T("agents.shared_context"));
        input.Visibility = contextExpanded ? Visibility.Visible : Visibility.Collapsed;
        void Update() { contextExpanded = toggle.IsChecked == true; input.Visibility = contextExpanded ? Visibility.Visible : Visibility.Collapsed; arrow.Source = design.Icon(contextExpanded ? "chevron-down" : "chevron-right", 10, design.ColorText("text-muted")).Source; }
        toggle.Checked += (_, _) => Update(); toggle.Unchecked += (_, _) => Update(); group.Children.Add(toggle); group.Children.Add(input); return group;
    }
    private void Render()
    {
        if (closed) return; rendering = true; choices.Clear(); CancelConnection();
        library.Children.Clear(); libraryHead.Children.Clear(); identity.Children.Clear(); inspector.Children.Clear(); headerActions.Children.Clear(); conversionButton = null; policyText = policyHelp = taskCountText = null; tools.Children.Clear();
        library.Spacing = 7; library.Padding = new Thickness(14, 0, 14, 16); libraryHead.Spacing = 0; libraryHead.Padding = new Thickness(14, 16, 14, 12);
        var leave = design.ToolButton(T("workflow_studio.back_to_settings"), "arrow-left", true); leave.MinHeight = 26; leave.Margin = new Thickness(0, -2, 0, 12); leave.Click += (_, _) => RequestLeave(back); libraryHead.Children.Add(leave);
        var libraryLabels = new StackPanel { Spacing = 3, Margin = new Thickness(0, 0, 0, 10) }; libraryLabels.Children.Add(Text(T("workflow_studio.library"), 13, true));
        var libraryCount = Text(F("workflow_studio.count", ("count", model.Templates.Length.ToString())), 10.5); libraryCount.Foreground = design.Brush("text-faint"); libraryLabels.Children.Add(libraryCount); libraryHead.Children.Add(libraryLabels);
        var create = design.ToolButton(T("workflow_studio.new"), "plus", true); design.ActionButton(create); create.MinHeight = 32; create.Padding = new Thickness(12, 6, 12, 6); create.Click += (_, _) => RequestLeave(OpenNew); create.HorizontalAlignment = HorizontalAlignment.Stretch; libraryHead.Children.Add(create);
        foreach (var template in model.Templates)
        {
            var templateGraph = NativeWorkflowGraph.Build((template["proposal"]?["tasks"] as JsonArray)?.OfType<JsonObject>() ?? []);
            var info = new StackPanel { Spacing = 5 };
            var entryTitle = new Grid { ColumnSpacing = 7, HorizontalAlignment = HorizontalAlignment.Left }; entryTitle.ColumnDefinitions.Add(new() { Width = new GridLength(1, GridUnitType.Star) }); entryTitle.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
            entryTitle.Children.Add(Leading(Text(S(template, "name"), 13, true), 1.5));
            if (template["builtin"]?.GetValue<bool>() == true) { var badge = Badge(T("workflow_studio.builtin"), true); badge.Background = design.Brush("clay-soft"); Grid.SetColumn(badge, 1); entryTitle.Children.Add(badge); }
            info.Children.Add(entryTitle);
            var description = Leading(Text(S(template, "description"), 10.5), 1.4); description.Foreground = design.Brush("text-faint"); description.MaxLines = 2; description.TextTrimming = TextTrimming.CharacterEllipsis; info.Children.Add(description);
            var structure = Leading(Text(F("workflow_studio.library_structure", ("tasks", templateGraph.Nodes.Length.ToString()), ("stages", templateGraph.Stages.Length.ToString())), 11), 1.6); structure.Foreground = design.Brush("text-muted"); info.Children.Add(structure);
            var entry = Button(S(template, "name"), () => RequestLeave(() => { model.Select(S(template, "id")); autoFit = true; Render(); }));
            design.QuietButton(entry); entry.Content = info; entry.HorizontalAlignment = HorizontalAlignment.Stretch; entry.HorizontalContentAlignment = HorizontalAlignment.Stretch; entry.Padding = new Thickness(10);
            entry.CornerRadius = new CornerRadius(9); entry.BorderThickness = new Thickness(1); entry.BorderBrush = new SolidColorBrush(Microsoft.UI.Colors.Transparent);
            if (S(template, "id") == model.SelectedId) { entry.Background = design.Brush("clay-soft"); entry.BorderBrush = Mix("clay", "border", .36); entry.Resources["ButtonBackgroundPointerOver"] = entry.Background; entry.Resources["ButtonBorderBrushPointerOver"] = entry.BorderBrush; entry.Resources["ButtonBackgroundPressed"] = entry.Background; entry.Resources["ButtonBorderBrushPressed"] = entry.BorderBrush; }
            library.Children.Add(entry);
        }
        if (model.Draft is not { } draft || model.Proposal is not { } proposal)
        { identity.Children.Add(Text(T("workflow_studio.edit"), 16, true)); inspector.Children.Add(Hint(L("新建工作流，或从模板开始。", "Create a workflow or start from a template."))); DrawGraph(); rendering = false; UpdateState(); return; }
        var studioLabel = Badge(T(model.SelectedId.Length == 0 ? "workflow_studio.new" : "workflow_studio.edit").ToUpperInvariant()); identity.Children.Add(studioLabel);
        var name = Input("", S(draft, "name"), value => model.Change(draft, "name", JsonValue.Create(value)));
        name.PlaceholderText = T("workflow_studio.untitled"); name.MaxLength = 100; name.FontWeight = Microsoft.UI.Text.FontWeights.SemiBold; InlineField(name); name.MinHeight = 0; name.Height = 26; design.BindTypography(name, size: 12); AutomationProperties.SetName(name, T("workflow_studio.name"));
        var nameField = new Grid { ColumnSpacing = 6 }; nameField.ColumnDefinitions.Add(new() { Width = new GridLength(1, GridUnitType.Star) }); nameField.ColumnDefinitions.Add(new() { Width = GridLength.Auto }); nameField.Children.Add(name);
        var pencil = design.Icon("edit", 13, design.ColorText("text-faint")); pencil.VerticalAlignment = VerticalAlignment.Center; pencil.Margin = new Thickness(0, 0, 8, 0); pencil.PointerPressed += (_, e) => { name.Focus(FocusState.Programmatic); e.Handled = true; }; Grid.SetColumn(pencil, 1); nameField.Children.Add(pencil); Grid.SetColumn(nameField, 1); identity.Children.Add(nameField);
        var descriptionInput = Input("", S(draft, "description"), value => model.Change(draft, "description", JsonValue.Create(value)));
        descriptionInput.PlaceholderText = T("workflow_studio.description_ph"); descriptionInput.MaxLength = 500; InlineField(descriptionInput); descriptionInput.MinHeight = 0; descriptionInput.Height = 24; design.BindTypography(descriptionInput, size: 12); descriptionInput.Foreground = design.Brush("text-muted"); AutomationProperties.SetName(descriptionInput, T("workflow_studio.description")); Grid.SetRow(descriptionInput, 1); Grid.SetColumnSpan(descriptionInput, 2); identity.Children.Add(descriptionInput);
        headerSummary = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 7, VerticalAlignment = VerticalAlignment.Center, Visibility = NativeWorkflowLayout.ShowHeaderSummary(ActualWidth) ? Visibility.Visible : Visibility.Collapsed };
        policyText = Text(model.ApprovalPolicyLabel, 10, true); policyText.Foreground = design.Brush("text-muted");
        headerSummary.Children.Add(RoundPill(new Border { Child = policyText, Background = design.Brush("bg-sunken"), Padding = new Thickness(8, 4, 8, 4) }));
        taskCountText = Text(model.TaskCountLabel, 10); taskCountText.Foreground = design.Brush("text-faint"); headerSummary.Children.Add(taskCountText); headerActions.Children.Add(headerSummary);
        saveButton = Button(model.Builtin ? T("workflow_studio.save_copy") : T("workflow_studio.save"), async () => { if (await model.SaveAsync() && !closed) Render(); }, true);
        saveButton.MinHeight = 30; saveButton.Padding = new Thickness(11, 7, 11, 7);
        if (saveButton.Content is TextBlock saveLabel) { design.BindTypography(saveLabel, 11.5); saveLabel.FontWeight = Microsoft.UI.Text.FontWeights.SemiBold; }
        if (!model.Builtin && model.SelectedId.Length > 0)
        {
            var delete = Button(T("workflow_studio.delete"), () => Confirm(F("workflow_studio.delete_description", ("name", S(draft, "name"))), async () => { if (await model.DeleteAsync() && !closed) Render(); }));
            delete.MinHeight = 30; delete.Padding = new Thickness(11, 7, 11, 7);
            delete.Background = Mix("err", "bg-elev", .08); delete.BorderBrush = Mix("err", "border", .25);
            if (delete.Content is TextBlock label) { design.BindTypography(label, 11.5); label.Foreground = design.Brush("err"); }
            headerActions.Children.Add(delete);
        }
        headerActions.Children.Add(saveButton);
        if (model.Tasks.Any(t => t["skill_ids"] is JsonArray { Count: > 0 }))
            headerActions.Children.Add(Button(T("workflow_studio.reconvert"), () => { if (!ConversionState.Busy) { ConversionState.SetLegacy(model.SelectedId, request: S(proposal, "goal")); OpenConversion(); } }));
        var zoomControls = new StackPanel { Orientation = Orientation.Horizontal };
        zoomControls.Children.Add(Icon(T("workflow_studio.graph_zoom_out"), "minus", () => { autoFit = false; zoom = Math.Max(25, zoom - 10); PositionCanvas(); }));
        zoomButton = Button(zoom + "%", () => { autoFit = true; zoom = graph.FitZoom(viewport.ActualWidth, viewport.ActualHeight); PositionCanvas(); }); design.QuietButton(zoomButton); zoomControls.Children.Add(zoomButton);
        zoomControls.Children.Add(Icon(T("workflow_studio.graph_zoom_in"), "plus", () => { autoFit = false; zoom = Math.Min(140, zoom + 10); PositionCanvas(); }));
        tools.Children.Add(new Border { Child = zoomControls, BorderBrush = design.Brush("border-strong"), BorderThickness = new Thickness(1), CornerRadius = new CornerRadius(8), Background = design.Brush("bg-elev") });
        var addMenu = new MenuFlyout { Placement = FlyoutPlacementMode.BottomEdgeAlignedRight };
        var menuStyle = new Style(typeof(MenuFlyoutPresenter)); menuStyle.Setters.Add(new Setter(FrameworkElement.MinWidthProperty, 225d)); addMenu.MenuFlyoutPresenterStyle = menuStyle;
        var independent = new MenuFlyoutItem { Text = T("workflow_studio.independent_task"), Icon = new ImageIcon { Source = design.Icon("branch", 16).Source } };
        independent.Click += (_, _) => OpenTask(null, model.NewTask()); addMenu.Items.Add(independent);
        var afterSelected = new MenuFlyoutItem { Text = T("workflow_studio.after_task"), Icon = new ImageIcon { Source = design.Icon("arrow-right", 16).Source } };
        afterSelected.Click += (_, _) => { if (model.CanAddAfterSelected && model.SelectedTaskId is { } id) OpenTask(null, model.NewTask(id)); }; addMenu.Items.Add(afterSelected);
        addMenu.Opening += (_, _) => afterSelected.IsEnabled = model.CanAddAfterSelected;
        menus.Register(addMenu); var add = design.ToolButton(T("workflow_studio.add_task"), "plus", true); design.ActionButton(add);
        var addContent = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 8 }; addContent.Children.Add(design.Icon("plus", 16, design.ColorText("clay-strong")));
        var addLabel = Text(T("workflow_studio.add_task"), 13, true); addLabel.Foreground = design.Brush("clay-strong"); addContent.Children.Add(addLabel); add.Content = addContent;
        add.Background = design.Brush("clay-soft"); add.BorderBrush = design.Brush("clay"); add.Resources["ButtonBackgroundPointerOver"] = design.Brush("clay-soft"); add.Flyout = addMenu; tools.Children.Add(add);
        if (model.Builtin) { var note = Leading(Text(T("workflow_studio.builtin_help"), 11), 1.45); note.Foreground = design.Brush("text-muted"); inspector.Children.Add(new Border { Child = note, Background = design.Brush("clay-soft"), BorderBrush = Mix("clay", "border", .3), BorderThickness = new Thickness(1), CornerRadius = new CornerRadius(8), Padding = new Thickness(11, 9, 11, 9), Margin = new Thickness(0, 11, 0, 0) }); }
        var goal = Input(T("agents.goal"), S(proposal, "goal"), value => model.Change(proposal, "goal", JsonValue.Create(value)), true); goal.MinHeight = 140; goal.PlaceholderText = T("agents.goal_ph"); goal.Header = Hint(T("agents.goal")); design.BindTypography(goal, size: 12); inspector.Children.Add(goal);
        var policy = Choice(T("agents.approval_policy"), S(proposal, "approval_policy"), [("review_all", T("agents.approval.review_all")), ("auto_safe", T("agents.approval.auto_safe"))], value => model.Change(proposal, "approval_policy", JsonValue.Create(value))); policy.Header = Hint(T("agents.approval_policy")); design.BindTypography(policy, size: 12); inspector.Children.Add(policy);
        policyHelp = Leading(Text(model.ApprovalPolicyHelp, 10.5), 1.5); policyHelp.Foreground = design.Brush("text-faint"); inspector.Children.Add(policyHelp);
        inspector.Children.Add(SharedContext(proposal));
        DrawGraph(); rendering = false; UpdateState();
    }
    private void UpdateState()
    {
        if (closed) return;
        if (policyText != null) policyText.Text = model.ApprovalPolicyLabel;
        if (policyHelp != null) policyHelp.Text = model.ApprovalPolicyHelp;
        if (taskCountText != null) taskCountText.Text = model.TaskCountLabel;
        if (ConversionState.RequestId != null)
        {
            var title = T(ConversionState.Busy ? "workflow_studio.conversion.view_progress" : "workflow_studio.conversion.view_result");
            if (conversionButton == null) { conversionButton = Button(title, OpenConversion); headerActions.Children.Add(conversionButton); }
            conversionButton.Content = title; AutomationProperties.SetName(conversionButton, title);
        }
        else if (conversionButton != null) { headerActions.Children.Remove(conversionButton); conversionButton = null; }
        contentHost.IsEnabled = !model.Busy && dialogs.Count == 0;
        status.Text = model.Busy ? L("正在处理…", "Working…") : model.Error ?? model.SavedMessage ?? (connecting == null ? model.SaveBlockReason ?? (model.ConversionSourceSha256 != null ? T("workflow_studio.portfolio.editor_notice") : "") : F("workflow_studio.graph_choose_target", ("node", connecting)));
        status.Visibility = status.Text.Length == 0 ? Visibility.Collapsed : Visibility.Visible;
        if (saveButton != null) { saveButton.IsEnabled = !model.Busy && model.SaveBlockReason == null; ToolTipService.SetToolTip(saveButton, model.SaveBlockReason); }
    }
    private void DrawGraph()
    {
        graph = NativeWorkflowGraph.Build(model.Tasks); canvas.Children.Clear(); nodeCards.Clear(); nodeRemoveButtons.Clear(); hoveredNodes.Clear(); edgePaths.Clear(); canvas.Width = graph.Width; canvas.Height = graph.Height;
        summary.Text = F("workflow_studio.structure", ("tasks", graph.Nodes.Length.ToString()), ("stages", graph.Stages.Length.ToString()), ("parallel", graph.Stages.Select(s => s.Count).DefaultIfEmpty(0).Max().ToString()));
        foreach (var stage in graph.Stages.Where(s => s.Count > 0))
        {
            var board = new Border { Width = NativeWorkflowGraph.NodeWidth + 32, Height = graph.Height - 24, CornerRadius = new CornerRadius(16), Background = new SolidColorBrush(design.Brush("bg-elev").Color) { Opacity = .55 }, BorderBrush = design.Brush("border"), BorderThickness = new Thickness(1), IsHitTestVisible = false };
            Canvas.SetLeft(board, stage.X - 16); Canvas.SetTop(board, 12); canvas.Children.Add(board);
            var heading = new Grid { Width = NativeWorkflowGraph.NodeWidth, ColumnSpacing = 8, IsHitTestVisible = false }; heading.ColumnDefinitions.Add(new() { Width = new GridLength(1, GridUnitType.Star) }); heading.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
            var title = Text(F("workflow_studio.graph_stage", ("number", (stage.Level + 1).ToString())).ToUpperInvariant(), 13, true); title.CharacterSpacing = 40; title.Foreground = design.Brush("text-muted"); title.VerticalAlignment = VerticalAlignment.Center; heading.Children.Add(title);
            if (stage.Count > 1) { var label = Text(F("workflow_studio.graph_parallel_count", ("count", stage.Count.ToString())), 11, true); label.Foreground = design.Brush("clay"); var count = RoundPill(new Border { Child = label, Background = design.Brush("clay-soft"), Padding = new Thickness(5, 2, 5, 2) }); Grid.SetColumn(count, 1); heading.Children.Add(count); }
            Canvas.SetLeft(heading, stage.X); Canvas.SetTop(heading, 30); canvas.Children.Add(heading);
        }
        foreach (var edge in graph.Edges)
        {
            var source = graph.Nodes.First(n => n.Id == edge.Source); var target = graph.Nodes.First(n => n.Id == edge.Target);
            var x1 = source.X + NativeWorkflowGraph.NodeWidth; var y1 = source.Y + NativeWorkflowGraph.NodeHeight / 2;
            var x2 = target.X; var y2 = target.Y + NativeWorkflowGraph.NodeHeight / 2; var bend = Math.Max(32, Math.Abs(x2 - x1) / 2);
            var figure = new PathFigure { StartPoint = new Point(x1, y1), IsClosed = false };
            figure.Segments.Add(new BezierSegment { Point1 = new Point(x1 + bend, y1), Point2 = new Point(x2 - bend, y2), Point3 = new Point(x2, y2) });
            var geometry = new PathGeometry(); geometry.Figures.Add(figure);
            var path = new Microsoft.UI.Xaml.Shapes.Path { Data = geometry, Stroke = design.Brush("text-muted"), StrokeThickness = 2 };
            // Match the WebView's 8-unit SVG marker at a 2-unit stroke, including refX=12.
            var arrow = new Polygon { Points = new PointCollection { new(x2 - 3.2, y2), new(x2 - 19.2, y2 - 8), new(x2 - 19.2, y2 + 8) }, Fill = design.Brush("text-muted"), IsHitTestVisible = false };
            edgePaths.Add((edge, path, arrow));
            var edgeMenu = new MenuFlyout(); var remove = new MenuFlyoutItem { Text = T("workflow_studio.graph_remove_edge") }; remove.Click += (_, _) => { model.Disconnect(edge.Source, edge.Target); Render(); }; edgeMenu.Items.Add(remove); menus.Register(edgeMenu); path.ContextFlyout = edgeMenu;
            ToolTipService.SetToolTip(path, edge.Source + " → " + edge.Target); canvas.Children.Add(path); canvas.Children.Add(arrow);
        }
        foreach (var node in graph.Nodes)
        {
            var nodeKey = model.TaskKeyAt(node.Index);
            var copy = new Grid { RowSpacing = 9, Padding = new Thickness(18, 16, 18, 16) };
            copy.RowDefinitions.Add(new() { Height = GridLength.Auto }); copy.RowDefinitions.Add(new() { Height = new GridLength(1, GridUnitType.Star) }); copy.RowDefinitions.Add(new() { Height = GridLength.Auto }); copy.RowDefinitions.Add(new() { Height = GridLength.Auto });
            var titleRow = new Grid { ColumnSpacing = 7, Padding = new Thickness(0, 0, 6, 0) }; titleRow.ColumnDefinitions.Add(new() { Width = new GridLength(1, GridUnitType.Star) }); titleRow.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
            var title = MonoText(node.Id, 14, true); title.FontWeight = Microsoft.UI.Text.FontWeights.Bold; title.MaxLines = 1; title.TextWrapping = TextWrapping.NoWrap; title.TextTrimming = TextTrimming.CharacterEllipsis; titleRow.Children.Add(title);
            var statusLabel = MonoText(T("workflow_studio.not_run"), 11); statusLabel.Foreground = design.Brush("text-faint");
            var notRun = new Border { Child = statusLabel, Background = design.Brush("bg-sunken"), CornerRadius = new CornerRadius(4), Padding = new Thickness(5, 2, 5, 2), VerticalAlignment = VerticalAlignment.Center }; Grid.SetColumn(notRun, 1); titleRow.Children.Add(notRun); copy.Children.Add(titleRow);
            var instruction = Text(S(node.Task, "instruction") is { Length: > 0 } details ? details : "…", 13); instruction.Foreground = design.Brush("text-muted"); instruction.MinHeight = 57; instruction.MaxLines = 3; instruction.TextTrimming = TextTrimming.CharacterEllipsis; instruction.VerticalAlignment = VerticalAlignment.Top; Grid.SetRow(instruction, 1); copy.Children.Add(instruction);
            var metadata = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 5 }; metadata.Children.Add(NodePill(Role(node.Task)));
            if (S(node.Task, "task_kind") != "run_activity") metadata.Children.Add(NodePill(F(NativeWorkflowGraph.CapabilityCount(node.Task) == 1 ? "workflow_studio.graph_capability" : "workflow_studio.graph_capabilities", ("count", NativeWorkflowGraph.CapabilityCount(node.Task).ToString())))); Grid.SetRow(metadata, 2); copy.Children.Add(metadata);
            var dependencies = Text(NativeWorkflowGraph.Dependencies(node.Task).Length == 0 ? T("workflow_studio.graph_root") : T("agents.task.dependencies") + ": " + string.Join(", ", NativeWorkflowGraph.Dependencies(node.Task)), 11); dependencies.Foreground = design.Brush("text-muted"); dependencies.MaxLines = 2; dependencies.TextTrimming = TextTrimming.CharacterEllipsis; Grid.SetRow(dependencies, 3); copy.Children.Add(dependencies);
            var card = new Button { Content = copy, Width = NativeWorkflowGraph.NodeWidth, Height = NativeWorkflowGraph.NodeHeight, Padding = new Thickness(0), HorizontalContentAlignment = HorizontalAlignment.Stretch, VerticalContentAlignment = VerticalAlignment.Stretch,
                CornerRadius = new CornerRadius(10), BorderThickness = new Thickness(1), BorderBrush = design.Brush("border-strong"), Background = design.Brush("bg-elev") };
            AutomationProperties.SetName(card, T("workflow_studio.edit_node") + " " + node.Id); ToolTipService.SetToolTip(card, T("workflow_studio.node_open_hint"));
            card.DoubleTapped += (_, e) => { SelectNode(nodeKey); OpenTask(nodeKey, node.Task); e.Handled = true; };
            card.Click += (_, _) => { if (connecting != null) CompleteConnection(node.Id); else SelectNode(nodeKey); };
            card.KeyDown += (_, e) => { if (e.Key == Windows.System.VirtualKey.Enter) { SelectNode(nodeKey); OpenTask(nodeKey, node.Task); e.Handled = true; } };
            nodeCards[nodeKey] = card;
            card.PointerEntered += (_, _) => { hoveredNodes.Add(nodeKey); UpdateNodeRemove(nodeKey); };
            card.PointerExited += (_, _) => { hoveredNodes.Remove(nodeKey); UpdateNodeRemove(nodeKey); };
            Canvas.SetLeft(card, node.X); Canvas.SetTop(card, node.Y); canvas.Children.Add(card);
            var incoming = Port(T("workflow_studio.graph_connect_target") + " " + node.Id, false, () => CompleteConnection(node.Id));
            Canvas.SetLeft(incoming, node.X - 13); Canvas.SetTop(incoming, node.Y + NativeWorkflowGraph.NodeHeight / 2 - 13); canvas.Children.Add(incoming);
            var outgoing = Port(F("workflow_studio.graph_connect_from", ("node", node.Id)), true, () => { if (!suppressPortClick) { connection.Begin(node.Id, node.X + NativeWorkflowGraph.NodeWidth, node.Y + NativeWorkflowGraph.NodeHeight / 2); UpdateState(); } });
            outgoing.AddHandler(PointerPressedEvent, new PointerEventHandler((_, e) => BeginConnection(node, outgoing, e)), true);
            Canvas.SetLeft(outgoing, node.X + NativeWorkflowGraph.NodeWidth - 13); Canvas.SetTop(outgoing, node.Y + NativeWorkflowGraph.NodeHeight / 2 - 13); canvas.Children.Add(outgoing);
            var remove = Icon(T("agents.task.remove") + " " + node.Id, "close", () => { model.RemoveTaskKey(nodeKey); Render(); });
            remove.Content = design.Icon("close", 12); remove.Width = remove.Height = remove.MinWidth = remove.MinHeight = 22; remove.Padding = new Thickness(0); remove.Opacity = 0;
            remove.PointerEntered += (_, _) => { hoveredNodes.Add(nodeKey); UpdateNodeRemove(nodeKey); }; remove.PointerExited += (_, _) => { hoveredNodes.Remove(nodeKey); UpdateNodeRemove(nodeKey); };
            remove.GotFocus += (_, _) => UpdateNodeRemove(nodeKey); remove.LostFocus += (_, _) => UpdateNodeRemove(nodeKey); nodeRemoveButtons[nodeKey] = remove;
            Canvas.SetLeft(remove, node.X + NativeWorkflowGraph.NodeWidth - 28); Canvas.SetTop(remove, node.Y + 6); canvas.Children.Add(remove);
        }
        canvas.Children.Add(connectionPreview);
        if (graph.Nodes.Length == 0) { var empty = Text(T("workflow_studio.graph_dblclick_add"), 16); Canvas.SetLeft(empty, 48); Canvas.SetTop(empty, 170); canvas.Children.Add(empty); }
        if (autoFit) zoom = graph.FitZoom(viewport.ActualWidth, viewport.ActualHeight); PositionCanvas(); RenderSelection();
    }
    private void PositionCanvas()
    {
        var scale = zoom / 100d; canvas.RenderTransform = new ScaleTransform { ScaleX = scale, ScaleY = scale };
        canvasHost.Width = Math.Max(viewport.ActualWidth, graph.Width * scale); canvasHost.Height = Math.Max(viewport.ActualHeight, graph.Height * scale);
        Canvas.SetLeft(canvas, Math.Max(0, (canvasHost.Width - graph.Width * scale) / 2)); Canvas.SetTop(canvas, Math.Max(0, (canvasHost.Height - graph.Height * scale) / 2));
        canvasHost.Background = new SolidColorBrush(Microsoft.UI.Colors.Transparent);
        // One geometry draws the grid; do not create a control for every background dot.
        var grid = new GeometryGroup();
        for (double y = 9; y < viewport.ActualHeight; y += 18)
            for (double x = 9; x < viewport.ActualWidth; x += 18) grid.Children.Add(new EllipseGeometry { Center = new Point(x, y), RadiusX = 1, RadiusY = 1 });
        dots.Data = grid;
        if (zoomButton != null) { zoomButton.Content = Text(zoom + "%", 12); AutomationProperties.SetName(zoomButton, T("workflow_studio.graph_fit") + $" ({zoom}%)"); }
        DrawMinimap();
    }
    private void SelectNode(long? key) { model.SelectTaskKey(key); RenderSelection(); }
    private void RenderSelection()
    {
        selectedSummary.Children.Clear();
        var selected = model.SelectedTaskId;
        var related = graph.Edges.Where(edge => edge.Source == selected || edge.Target == selected).SelectMany(edge => new[] { edge.Source, edge.Target }).Append(selected).ToHashSet();
        foreach (var (key, card) in nodeCards)
        {
            var id = S(model.TaskForKey(key), "id"); var isSelected = key == model.SelectedTaskKey;
            card.BorderBrush = design.Brush(isSelected ? "clay" : "border-strong");
            card.Background = design.Brush(isSelected ? "surface-hover" : "bg-elev");
            card.Opacity = model.SelectedTaskKey != null && !isSelected && !related.Contains(id) ? .5 : 1;
        }
        foreach (var key in nodeRemoveButtons.Keys) UpdateNodeRemove(key);
        foreach (var (edge, path, arrow) in edgePaths) { path.Stroke = design.Brush(edge.Source == selected || edge.Target == selected ? "clay" : "text-muted"); arrow.Fill = path.Stroke; }
        if (model.SelectedTask is { } task)
        {
            var heading = new Grid { ColumnSpacing = 8 }; heading.ColumnDefinitions.Add(new() { Width = new GridLength(1, GridUnitType.Star) }); heading.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
            var title = new StackPanel { Spacing = 2 }; var caption = Text(T("workflow_studio.graph_selected").ToUpperInvariant(), 11, true); caption.Foreground = design.Brush("text-faint"); title.Children.Add(caption);
            var taskId = MonoText(S(task, "id"), 12, true); taskId.FontWeight = Microsoft.UI.Text.FontWeights.Bold; taskId.TextWrapping = TextWrapping.NoWrap; taskId.TextTrimming = TextTrimming.CharacterEllipsis; title.Children.Add(taskId); heading.Children.Add(title);
            var close = Icon(T("workflow_studio.close_details"), "close", () => SelectNode(null)); close.MinWidth = close.MinHeight = 30; close.Width = close.Height = 30; close.Padding = new Thickness(6); close.BorderThickness = new Thickness(1); close.BorderBrush = design.Brush("border"); close.Background = design.Brush("bg-elev"); close.CornerRadius = new CornerRadius(6); close.VerticalAlignment = VerticalAlignment.Top; Grid.SetColumn(close, 1); heading.Children.Add(close); heading.Margin = new Thickness(0, 0, 0, 10); selectedSummary.Children.Add(heading);
            var instruction = Leading(Hint(S(task, "instruction")), 1.6); instruction.MaxLines = 5; instruction.TextTrimming = TextTrimming.CharacterEllipsis; selectedSummary.Children.Add(instruction);
            var metadata = new StackPanel { Spacing = 4, Margin = new Thickness(0, 14, 0, 14) };
            var roleLabel = Text(T("agents.task.specialist"), 12); roleLabel.Foreground = design.Brush("text-faint"); metadata.Children.Add(roleLabel);
            var role = Text(Role(task), 12); role.Margin = new Thickness(0, 0, 0, 8); metadata.Children.Add(role);
            var dependencyLabel = Text(T("agents.task.dependencies"), 12); dependencyLabel.Foreground = design.Brush("text-faint"); metadata.Children.Add(dependencyLabel);
            metadata.Children.Add(Text(NativeWorkflowGraph.Dependencies(task).Length == 0 ? T("workflow_studio.graph_root") : string.Join(", ", NativeWorkflowGraph.Dependencies(task)), 12)); selectedSummary.Children.Add(metadata);
            var edit = design.ToolButton(T("workflow_studio.edit_node"), "edit", true); design.ActionButton(edit); edit.MinHeight = 28; edit.Padding = new Thickness(10, 5, 10, 5); edit.HorizontalAlignment = HorizontalAlignment.Left; edit.Click += (_, _) => OpenTask(model.SelectedTaskKey, task); selectedSummary.Children.Add(edit);
            var hint = Text(T("workflow_studio.node_open_hint"), 11); hint.Foreground = design.Brush("text-faint"); hint.Margin = new Thickness(0, 8, 0, 0); selectedSummary.Children.Add(hint);
        }
        ResizeInspector(); DrawMinimap();
    }
    private void ResizeInspector()
    {
        var selected = model.SelectedTaskKey != null;
        configurationPane.MaxHeight = double.PositiveInfinity;
        configurationScroll.MaxHeight = NativeWorkflowLayout.ConfigurationBodyMaxHeight(ActualWidth, ActualHeight, selected);
        inspectorLayout.RowDefinitions[0].Height = GridLength.Auto;
        if (NativeWorkflowLayout.StackPanels(ActualWidth))
        {
            selectedScroll.MaxHeight = 440;
            inspectorLayout.RowDefinitions[1].Height = selected ? GridLength.Auto : new GridLength(0);
            selectedScroll.Visibility = selected ? Visibility.Visible : Visibility.Collapsed; return;
        }
        selectedScroll.MaxHeight = double.PositiveInfinity;
        inspectorLayout.RowDefinitions[1].Height = selected ? new GridLength(1, GridUnitType.Star) : new GridLength(0);
        selectedScroll.Visibility = selected ? Visibility.Visible : Visibility.Collapsed;
    }
    private void ResizeColumns()
    {
        if (headerSummary != null) headerSummary.Visibility = NativeWorkflowLayout.ShowHeaderSummary(ActualWidth) ? Visibility.Visible : Visibility.Collapsed;
        var widths = NativeWorkflowLayout.ForWidth(ActualWidth, libraryWidth, inspectorWidth);
        var stacked = NativeWorkflowLayout.StackPanels(ActualWidth);
        studioScroll.VerticalScrollBarVisibility = stacked ? ScrollBarVisibility.Auto : ScrollBarVisibility.Disabled;
        studioScroll.VerticalScrollMode = stacked ? ScrollMode.Enabled : ScrollMode.Disabled;
        layout.ColumnDefinitions[0].Width = stacked ? new GridLength(1, GridUnitType.Star) : new GridLength(widths.LibraryWidth);
        layout.ColumnDefinitions[1].Width = stacked ? new GridLength(0) : new GridLength(1, GridUnitType.Star);
        layout.RowDefinitions[0].Height = stacked ? new GridLength(280) : new GridLength(1, GridUnitType.Star);
        Grid.SetColumn(editor, stacked ? 0 : 1); Grid.SetRow(editor, stacked ? 1 : 0); editor.MinHeight = stacked ? 680 : 0;
        editor.RowDefinitions[2].Height = stacked ? GridLength.Auto : new GridLength(1, GridUnitType.Star);
        workspace.ColumnDefinitions[1].Width = stacked ? new GridLength(0) : new GridLength(widths.InspectorWidth);
        workspace.RowDefinitions[0].Height = stacked ? new GridLength(620) : new GridLength(1, GridUnitType.Star);
        Grid.SetColumn(inspectorFrame, stacked ? 0 : 1); Grid.SetRow(inspectorFrame, stacked ? 1 : 0);
        libraryFrame.BorderThickness = stacked ? new Thickness(0, 0, 0, 1) : new Thickness(0, 0, 1, 0);
        inspectorFrame.BorderThickness = stacked ? new Thickness(0, 1, 0, 0) : new Thickness(1, 0, 0, 0);
        libraryResizer.Visibility = inspectorResizer.Visibility = stacked ? Visibility.Collapsed : Visibility.Visible;
        ResizeInspector();
    }
    private Thumb Resizer(string label, bool right)
    {
        var handle = new Thumb { Width = 6, HorizontalAlignment = right ? HorizontalAlignment.Left : HorizontalAlignment.Right, VerticalAlignment = VerticalAlignment.Stretch, IsTabStop = true, Background = new SolidColorBrush(Microsoft.UI.Colors.Transparent) };
        AutomationProperties.SetName(handle, label); ToolTipService.SetToolTip(handle, label + L("；左右方向键调整", "; use Left and Right to resize"));
        void Resize(double delta)
        {
            var current = right ? workspace.ColumnDefinitions[1].ActualWidth : layout.ColumnDefinitions[0].ActualWidth;
            var next = current + (right ? -delta : delta);
            if (right) inspectorWidth = next; else libraryWidth = next;
            ResizeColumns();
        }
        handle.DragDelta += (_, e) => Resize(e.HorizontalChange);
        handle.KeyDown += (_, e) => { if (e.Key is Windows.System.VirtualKey.Left or Windows.System.VirtualKey.Right) { Resize(e.Key == Windows.System.VirtualKey.Left ? -20 : 20); e.Handled = true; } };
        handle.PointerEntered += (_, _) => handle.Background = design.Brush("clay");
        handle.PointerExited += (_, _) => handle.Background = new SolidColorBrush(Microsoft.UI.Colors.Transparent);
        return handle;
    }
    private void DrawMinimap()
    {
        minimap.Children.Clear(); minimapFrame.Visibility = graph.ShowMinimap(zoom) ? Visibility.Visible : Visibility.Collapsed;
        var scale = Math.Min(132 / graph.Width, 86 / graph.Height); var left = (132 - graph.Width * scale) / 2; var top = (86 - graph.Height * scale) / 2;
        foreach (var node in graph.Nodes)
        {
            var item = new Rectangle { Width = NativeWorkflowGraph.NodeWidth * scale, Height = NativeWorkflowGraph.NodeHeight * scale, RadiusX = 2, RadiusY = 2,
                Fill = design.Brush(model.TaskKeyAt(node.Index) == model.SelectedTaskKey ? "clay" : "border-strong") };
            Canvas.SetLeft(item, left + node.X * scale); Canvas.SetTop(item, top + node.Y * scale); minimap.Children.Add(item);
        }
    }
    private Border Badge(string label, bool accent = false)
    {
        var text = Text(label, 10); text.Foreground = design.Brush(accent ? "clay" : "text-muted");
        return new Border { Child = text, Background = design.Brush("bg-sunken"), CornerRadius = new CornerRadius(5), Padding = new Thickness(5, 2, 5, 2), VerticalAlignment = VerticalAlignment.Center };
    }
    private Border NodePill(string label)
    {
        var text = Text(label, 11); text.Foreground = design.Brush("text-muted"); text.TextWrapping = TextWrapping.NoWrap; text.TextTrimming = TextTrimming.CharacterEllipsis;
        return RoundPill(new Border { Child = text, MaxWidth = 122, Background = design.Brush("bg-sunken"), BorderBrush = design.Brush("border-strong"), BorderThickness = new Thickness(1), Padding = new Thickness(7, 3, 7, 3) });
    }
    private static Border RoundPill(Border pill)
    {
        // XAML scales an oversized radius into an ellipse; CSS clamps it to half the height.
        pill.SizeChanged += (_, e) => pill.CornerRadius = new CornerRadius(Math.Min(e.NewSize.Width, e.NewSize.Height) / 2);
        return pill;
    }
    private void UpdateNodeRemove(long key)
    { if (nodeRemoveButtons.TryGetValue(key, out var button)) button.Opacity = model.SelectedTaskKey == key || hoveredNodes.Contains(key) || button.FocusState != FocusState.Unfocused ? 1 : 0; }
    private void InlineField(TextBox input)
    {
        input.TextWrapping = TextWrapping.NoWrap; input.Background = new SolidColorBrush(Microsoft.UI.Colors.Transparent);
        input.BorderBrush = new SolidColorBrush(Microsoft.UI.Colors.Transparent); input.Padding = new Thickness(6, 3, 6, 3);
        input.GotFocus += (_, _) => { input.BorderBrush = design.Brush("clay"); input.Background = design.Brush("bg-app"); };
        input.LostFocus += (_, _) => { input.BorderBrush = new SolidColorBrush(Microsoft.UI.Colors.Transparent); input.Background = new SolidColorBrush(Microsoft.UI.Colors.Transparent); };
    }
    private Button Port(string label, bool outgoing, Action action)
    {
        var button = Button(label, action); design.QuietButton(button); button.Width = button.Height = button.MinWidth = button.MinHeight = 26; button.Padding = new Thickness(0);
        button.Content = new Border { Width = 16, Height = 16, BorderBrush = design.Brush(outgoing ? "clay" : "border-strong"), BorderThickness = new Thickness(1), CornerRadius = new CornerRadius(999), Child = new Ellipse { Width = 14, Height = 14, Fill = design.Brush(outgoing ? "clay" : "text-faint"), Stroke = design.Brush("bg-elev"), StrokeThickness = 2 } };
        ToolTipService.SetToolTip(button, label); return button;
    }
    public void RequestLeave(Action action)
    { if (model.Busy) return; if (model.HasChanges) Confirm(L("放弃尚未保存的工作流修改？", "Discard unsaved workflow changes?"), action); else action(); }
    public void HandleEscape()
    {
        if (menus.CloseTop()) return;
        if (choices.LastOrDefault(c => c.IsDropDownOpen) is { } combo) { combo.IsDropDownOpen = false; return; }
        if (dialogs.Count > 0) { CloseDialog(); return; }
        if (connecting != null) { CancelConnection(); return; }
        if (model.SelectedTaskKey != null) { SelectNode(null); return; }
        RequestLeave(back);
    }
    public void Dispose() { if (closed) return; CancelConnection(); CloseDialogs(); closed = true; ConversionState.Changed -= RefreshConversion; menus.Dispose(); model.Changed -= UpdateState; model.Dispose(); }
}
