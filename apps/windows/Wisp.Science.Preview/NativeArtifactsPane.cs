using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Media;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

namespace Wisp.Science.Preview;

/// <summary>The current artifact list stays mounted while its preview changes.</summary>
internal sealed class NativeArtifactsPane : UserControl, IDisposable
{
    private readonly WispDesign design;
    private readonly NativeGeneratedPreviewModel previewModel;
    private readonly Func<NativePanelArtifact, Task>? openRegistered;
    private readonly Func<NativeArtifactEntry, NativeArtifactFileAction, Task>? fileAction;
    private readonly Func<NativeArtifactEntry, NativeArtifactFileAction, bool>? canFileAction;
    private readonly Func<string, NativeDocumentSelection, bool>? quoteDocument;
    private MenuFlyout? openMenu;
    private bool actionBusy;
    private readonly Grid root = new();
    private readonly StackPanel groups = new() { Spacing = 12 };
    private readonly ScrollViewer list;
    private readonly Border previewHost = new();
    private readonly ContentControl emptyHost = new() { Visibility = Visibility.Collapsed,
        HorizontalContentAlignment = HorizontalAlignment.Center, VerticalContentAlignment = VerticalAlignment.Center };
    private readonly Dictionary<string, Border> tiles = [];
    private readonly HashSet<string> collapsed = [];
    private readonly Button listMode, gridMode;
    private NativeArtifactEntry[] entries = [];
    private NativeGeneratedPreview? preview;
    private string style = "", error = "";
    private bool grid, loading, disposed, restore;
    private double offset;
    public FrameworkElement ViewModes { get; }
    public NativeArtifactsPane(WispDesign design,
        Func<NativeGeneratedOutput, CancellationToken, Task<NativePanelFileContent>>? read = null,
        Func<NativePanelArtifact, Task>? openRegistered = null,
        Func<NativeArtifactEntry, NativeArtifactFileAction, Task>? fileAction = null,
        Func<NativeArtifactEntry, NativeArtifactFileAction, bool>? canFileAction = null,
        Func<NativePanelArtifact, CancellationToken, Task<NativePanelFileContent>>? readRegistered = null,
        Func<string, NativeDocumentSelection, bool>? quoteDocument = null)
    {
        this.design = design; this.openRegistered = openRegistered; previewModel = new(read, readRegistered);
        this.fileAction = fileAction; this.canFileAction = canFileAction; this.quoteDocument = quoteDocument;
        root.RowDefinitions.Add(new()); root.RowDefinitions.Add(new() { Height = new GridLength(0) });
        list = new() { Content = groups, Padding = new(12), HorizontalScrollBarVisibility = ScrollBarVisibility.Disabled,
            HorizontalContentAlignment = HorizontalAlignment.Stretch, Background = design.Brush("bg-sunken") };
        root.Children.Add(list); root.Children.Add(emptyHost); Grid.SetRow(previewHost, 1); root.Children.Add(previewHost);
        previewHost.BorderBrush = design.Brush("border"); previewHost.BorderThickness = new(0, 1, 0, 0);
        var modes = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 2 };
        listMode = Mode("right.view.list", "list", false); gridMode = Mode("right.view.grid", "grid", true);
        modes.Children.Add(listMode); modes.Children.Add(gridMode);
        ViewModes = new Border { Child = modes, Padding = new(2), CornerRadius = new(5), BorderThickness = new(1),
            BorderBrush = design.Brush("border"), Background = design.Brush("bg-sunken"), VerticalAlignment = VerticalAlignment.Center };
        list.LayoutUpdated += (_, _) => { if (restore) { restore = false; list.ChangeView(null, Math.Min(offset, list.ScrollableHeight), null, true); } };
        SizeChanged += (_, _) => Layout(); previewModel.Changed += PreviewChanged;
        Content = root; RenderGroups(); Layout();
    }
    private Button Mode(string label, string icon, bool value)
    {
        var button = design.ToolButton(design.T(label), icon); button.Width = 26; button.Height = 24;
        button.MinWidth = button.MinHeight = 0; button.Padding = new(0); button.Content = design.Icon(icon, 15);
        button.Click += (_, _) => { if (grid == value) return; grid = value; RenderGroups(); };
        return button;
    }
    public bool Update(NativeGeneratedArtifacts generated, IEnumerable<NativePanelArtifact> registered, string projectRoot,
        bool isLoading = false, string? readError = null)
    {
        if (disposed) return false;
        var current = NativeArtifactCatalog.Collect(generated.Items, registered, projectRoot, design.Localization);
        var currentStyle = design.Localization.Locale + "/" + design.Dark + "/" + design.Typography;
        var appearanceChanged = style != currentStyle;
        var changed = !entries.SequenceEqual(current) || loading != isLoading || error != (readError ?? "") || style != currentStyle;
        entries = current; loading = isLoading; error = readError ?? ""; style = currentStyle;
        previewModel.BindCatalog(entries);
        if (changed) RenderGroups();
        if (appearanceChanged && preview != null)
        {
            preview.Dispose(); preview = null; previewHost.Child = null; PreviewChanged();
        }
        return changed;
    }
    public async Task<bool> OpenAsync(NativeGeneratedOutput output, CancellationToken token = default)
    {
        if (token.IsCancellationRequested || entries.FirstOrDefault(item => item.Id == output.Id) is not { } entry) return false;
        return await OpenEntryAsync(entry, token);
    }
    private async Task<bool> OpenEntryAsync(NativeArtifactEntry entry, CancellationToken token = default)
    {
        if (disposed || token.IsCancellationRequested) return false;
        if (NativeArtifactFileActions.OpensInModal(entry) && Available(entry, NativeArtifactFileAction.Viewer))
        { await PerformAsync(entry, NativeArtifactFileAction.Viewer); return true; }
        if (NativeArtifactFileActions.OpensInModal(entry) && entry.Registered is { } registered && openRegistered != null)
        { await openRegistered(registered); return true; }
        return await previewModel.OpenAsync(entry.Id, token);
    }
    public void ClosePreview() => previewModel.Close();
    public bool HandleEscape()
    {
        if (openMenu is { } menu) { menu.Hide(); return true; }
        return preview?.HandleEscape() == true;
    }
    private bool Available(NativeArtifactEntry entry, NativeArtifactFileAction action)
        => !disposed && fileAction != null && entry.DisplayPath != null && (canFileAction?.Invoke(entry, action) ?? true);
    private async Task PerformAsync(NativeArtifactEntry entry, NativeArtifactFileAction action)
    {
        if (actionBusy || entries.FirstOrDefault(item => item.Id == entry.Id) is not { } current || !Available(current, action)) return;
        openMenu?.Hide(); actionBusy = true;
        try { await fileAction!(current, action); }
        finally { actionBusy = false; }
    }
    private void PreviewChanged()
    {
        if (disposed) return;
        if (previewModel.Selected == null) { preview?.Dispose(); preview = null; previewHost.Child = null; }
        else if (preview == null) previewHost.Child = preview = new(previewModel, design, previewModel.Close,
            quoteDocument == null ? null : (path, selection) => !disposed && IsLoaded && quoteDocument(path, selection));
        foreach (var (id, border) in tiles) StyleTile(border, id);
        Layout();
    }
    private void Layout()
    {
        var selected = previewModel.Selected != null;
        root.RowDefinitions[0].Height = selected ? GridLength.Auto : new GridLength(1, GridUnitType.Star);
        root.RowDefinitions[1].Height = selected ? new GridLength(1, GridUnitType.Star) : new GridLength(0);
        list.MaxHeight = selected ? Math.Max(60, Math.Min(240, (XamlRoot?.Size.Height ?? ActualHeight) * .34)) : double.PositiveInfinity;
        previewHost.Visibility = selected ? Visibility.Visible : Visibility.Collapsed;
    }
    private void StyleTile(Border border, string id)
    {
        var active = previewModel.Selected?.Id == id;
        border.BorderBrush = design.Brush(active ? "clay" : "border-strong");
        border.Background = design.Brush(active ? "clay-soft" : "bg-elev");
    }
    private void RenderGroups()
    {
        if (disposed) return;
        openMenu?.Hide(); openMenu = null;
        offset = list.VerticalOffset; restore = true; groups.Children.Clear(); tiles.Clear();
        foreach (var (button, selected) in new[] { (listMode, !grid), (gridMode, grid) })
        {
            button.Background = design.Brush(selected ? "bg-elev" : "bg-sunken");
            AutomationProperties.SetItemStatus(button, selected ? design.L("已选中", "Selected") : "");
        }
        if (loading) groups.Children.Add(new ProgressBar { IsIndeterminate = true, Height = 3 });
        if (error.Length > 0) groups.Children.Add(design.Text(error, 12));
        var empty = entries.Length == 0 && !loading && error.Length == 0;
        emptyHost.Visibility = empty ? Visibility.Visible : Visibility.Collapsed;
        list.Visibility = empty ? Visibility.Collapsed : Visibility.Visible;
        ViewModes.Visibility = entries.Length == 0 ? Visibility.Collapsed : Visibility.Visible;
        root.Background = design.Brush(empty ? "bg-elev" : "bg-sunken");
        if (empty) { emptyHost.Content = new NativePanelEmptyState(design, "artifacts"); return; }
        var first = true;
        foreach (var group in NativeArtifactCatalog.Groups(entries, design.Localization))
        {
            var section = new StackPanel { Spacing = 6 };
            var items = new Grid { ColumnSpacing = grid ? 8 : 0, RowSpacing = grid ? 8 : 6 };
            var lastColumns = 0;
            void ArrangeItems(double width)
            {
                var columns = grid ? Math.Max(1, (int)Math.Floor((width + 8) / 136)) : 1;
                if (columns == lastColumns) return; lastColumns = columns;
                items.ColumnDefinitions.Clear(); items.RowDefinitions.Clear();
                for (var c = 0; c < columns; c++) items.ColumnDefinitions.Add(new());
                for (var i = 0; i < group.Items.Length; i++)
                {
                    if (i % columns == 0) items.RowDefinitions.Add(new() { Height = GridLength.Auto });
                    var element = (FrameworkElement)items.Children[i]; Grid.SetRow(element, i / columns); Grid.SetColumn(element, i % columns);
                }
            }
            foreach (var entry in group.Items) items.Children.Add(Tile(entry));
            items.SizeChanged += (_, e) => ArrangeItems(e.NewSize.Width); ArrangeItems(Math.Max(128, ActualWidth - 24));
            var heading = new Grid { ColumnSpacing = 6 };
            heading.ColumnDefinitions.Add(new() { Width = GridLength.Auto }); heading.ColumnDefinitions.Add(new()); heading.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
            var arrow = design.Icon("chevron-down", 12); arrow.RenderTransformOrigin = new(.5, .5); heading.Children.Add(arrow);
            var name = design.Text(group.Label.ToUpperInvariant(), 11); name.CharacterSpacing = 40; name.FontWeight = Microsoft.UI.Text.FontWeights.SemiBold;
            name.TextWrapping = TextWrapping.NoWrap; name.TextTrimming = TextTrimming.CharacterEllipsis; name.Foreground = design.Brush("text-faint"); Grid.SetColumn(name, 1); heading.Children.Add(name);
            var count = new Border { Child = design.Text(group.Items.Length.ToString(), 10), Padding = new(6, 1, 6, 1), CornerRadius = new(99),
                BorderThickness = new(1), BorderBrush = design.Brush("border-strong"), Background = design.Brush("bg-elev") }; Grid.SetColumn(count, 2); heading.Children.Add(count);
            var toggle = new Button { Content = heading, HorizontalAlignment = HorizontalAlignment.Stretch, HorizontalContentAlignment = HorizontalAlignment.Stretch };
            design.QuietButton(toggle); toggle.MinHeight = 0; toggle.Padding = new(2); AutomationProperties.SetName(toggle, group.Label + " · " + group.Items.Length);
            void Show()
            {
                var hidden = collapsed.Contains(group.Key); items.Visibility = hidden ? Visibility.Collapsed : Visibility.Visible;
                arrow.RenderTransform = new RotateTransform { Angle = hidden ? -90 : 0 };
                AutomationProperties.SetItemStatus(toggle, hidden ? design.L("已折叠", "Collapsed") : design.L("已展开", "Expanded"));
            }
            toggle.Click += (_, _) => { if (!collapsed.Add(group.Key)) collapsed.Remove(group.Key); Show(); };
            section.Children.Add(toggle); section.Children.Add(items); Show();
            groups.Children.Add(new Border { Child = section, Padding = new(0, first ? 0 : 12, 0, 0), BorderThickness = new(0, first ? 0 : 1, 0, 0), BorderBrush = design.Brush("border") }); first = false;
        }
    }
    private FrameworkElement Tile(NativeArtifactEntry entry)
    {
        var text = new StackPanel { Spacing = 2 };
        var name = design.Text(entry.Name, 12.5); name.FontWeight = Microsoft.UI.Text.FontWeights.SemiBold;
        name.TextWrapping = grid ? TextWrapping.Wrap : TextWrapping.NoWrap; name.TextTrimming = TextTrimming.CharacterEllipsis;
        var meta = design.Text(entry.Metadata, 11); meta.TextWrapping = TextWrapping.NoWrap; meta.TextTrimming = TextTrimming.CharacterEllipsis; meta.Foreground = design.Brush("text-faint");
        text.Children.Add(name); text.Children.Add(meta);
        var content = new Grid { ColumnSpacing = 10, RowSpacing = 8 };
        content.ColumnDefinitions.Add(new()); if (!grid) content.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
        content.Children.Add(text);
        var badgeText = design.Text(entry.Kind.ToUpperInvariant(), 9); badgeText.FontWeight = Microsoft.UI.Text.FontWeights.SemiBold;
        badgeText.Foreground = design.Brush(entry.Kind == "table" ? "clay-strong" : "text-faint");
        var badge = new Border { Child = badgeText, Padding = new(6, 2, 6, 2), CornerRadius = new(4),
            BorderThickness = new(entry.Kind == "table" ? 0 : 1), BorderBrush = design.Brush("border"),
            Background = design.Brush(entry.Kind == "table" ? "clay-soft" : "bg-elev"), VerticalAlignment = VerticalAlignment.Center, HorizontalAlignment = HorizontalAlignment.Left };
        if (grid) { content.RowDefinitions.Add(new() { Height = GridLength.Auto }); content.RowDefinitions.Add(new() { Height = GridLength.Auto }); Grid.SetRow(badge, 1); }
        else Grid.SetColumn(badge, 1); content.Children.Add(badge);
        var button = new Button { Content = content, HorizontalAlignment = HorizontalAlignment.Stretch, HorizontalContentAlignment = HorizontalAlignment.Stretch,
            VerticalAlignment = VerticalAlignment.Stretch };
        design.QuietButton(button); button.Padding = grid ? new(12) : new(12, 10, 12, 10); button.MinHeight = grid ? 62 : 0;
        AutomationProperties.SetName(button, entry.Name + " · " + entry.Metadata); AutomationProperties.SetAutomationId(button, "artifact-tile");
        ToolTipService.SetToolTip(button, entry.DisplayPath ?? entry.Name);
        button.Click += async (_, _) =>
        {
            if (disposed || entries.FirstOrDefault(item => item.Id == entry.Id) is not { } current) return;
            await OpenEntryAsync(current);
        };
        var tile = new Grid(); tile.Children.Add(button);
        if (entry.DisplayPath != null && fileAction != null)
        {
            var actions = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 1, Margin = grid ? new(0, 4, 4, 0) : new(0, 0, 6, 0),
                HorizontalAlignment = HorizontalAlignment.Right, VerticalAlignment = grid ? VerticalAlignment.Top : VerticalAlignment.Center };
            if (!grid) { tile.ColumnDefinitions.Add(new()); tile.ColumnDefinitions.Add(new() { Width = GridLength.Auto }); Grid.SetColumn(actions, 1); }
            else { button.Padding = new(12, 30, 12, 12); }
            Button Tool(string key, string icon)
            {
                var tool = design.ToolButton(design.T(key), icon); tool.Width = tool.Height = 24; tool.MinWidth = tool.MinHeight = 0; tool.Padding = new(0);
                tool.Content = design.Icon(icon, 15); AutomationProperties.SetName(tool, design.T(key) + ": " + entry.Name); return tool;
            }
            var download = Tool("artifact.download", "download"); download.IsEnabled = Available(entry, NativeArtifactFileAction.Download);
            download.Click += async (_, _) => await PerformAsync(entry, NativeArtifactFileAction.Download); actions.Children.Add(download);
            var more = Tool("artifact.more", "more"); var menu = new MenuFlyout { Placement = Microsoft.UI.Xaml.Controls.Primitives.FlyoutPlacementMode.BottomEdgeAlignedRight };
            foreach (var option in NativeArtifactFileActions.Items)
            {
                var action = option.Action; var item = new MenuFlyoutItem { Text = design.T(option.Label), FontSize = design.FontSize(12.5) };
                item.Click += async (_, _) => await PerformAsync(entry, action); menu.Items.Add(item);
            }
            menu.Opening += (_, _) => {
                if (disposed) { menu.Hide(); return; }
                openMenu?.Hide(); openMenu = menu;
                for (var i = 0; i < menu.Items.Count; i++)
                {
                    menu.Items[i].IsEnabled = !actionBusy && Available(entry, NativeArtifactFileActions.Items[i].Action);
                    ((MenuFlyoutItem)menu.Items[i]).FontSize = design.FontSize(12.5);
                }
            };
            menu.Closed += (_, _) => { if (openMenu == menu) openMenu = null; };
            more.Flyout = menu; actions.Children.Add(more); tile.Children.Add(actions);
        }
        var border = new Border { Child = tile, BorderThickness = new(1), CornerRadius = new(7) }; tiles[entry.Id] = border; StyleTile(border, entry.Id); return border;
    }
    public void Dispose()
    {
        if (disposed) return; disposed = true; openMenu?.Hide(); previewModel.Changed -= PreviewChanged; previewModel.Close(); preview?.Dispose(); Content = null;
    }
}
