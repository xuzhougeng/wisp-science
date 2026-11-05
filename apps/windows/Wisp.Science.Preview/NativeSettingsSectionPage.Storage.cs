using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Controls.Primitives;
using Microsoft.UI.Xaml.Input;
using Wisp.ProjectBrowser;

namespace Wisp.Science.Preview;

internal sealed partial class NativeSettingsSectionPage
{
    public NativeStorageBrowser StorageBrowser { private get; init; } = new();
    private CancellationTokenSource? storageScan;
    private string? storageError;
    private Grid? storageTiles;
    private FocusState storageSelectionFocus;
    private string? storageFocusProject;

    private async Task RefreshStorageAsync()
    {
        if (settingsClient == null || storageScan != null || model.Closed) return;
        using var scan = new CancellationTokenSource();
        storageScan = scan;
        try
        {
            storageError = null;
            Render();
            await StorageBrowser.RefreshAsync(settingsClient, settingsProjectId, scan.Token);
        }
        catch (OperationCanceledException) when (scan.IsCancellationRequested) { return; }
        catch (Exception ex) { storageError = ex.Message; }
        finally { storageScan = null; }
        if (model.Closed || scan.IsCancellationRequested) return;
        // Only restore focus if it was inside a project tile when the read finished.
        if (storageTiles != null && XamlRoot != null && FocusManager.GetFocusedElement(XamlRoot) is ToggleButton { Tag: string id } tile
            && tile.Parent == storageTiles)
        {
            storageSelectionFocus = tile.FocusState;
            storageFocusProject = StorageBrowser.Snapshot?.Select(id).ProjectId;
        }
        Render();
    }
    private void Storage()
    {
        storageTiles = null;
        Body.Padding = new Thickness(24, 18, 24, 22); Form.Spacing = 12;
        var intro = new StackPanel { Spacing = 3, VerticalAlignment = VerticalAlignment.Center };
        var title = Design.Text(T("settings.storage.data_dir"), 14); title.FontWeight = Microsoft.UI.Text.FontWeights.SemiBold;
        intro.Children.Add(title); intro.Children.Add(StorageText(T("settings.storage.data_dir_hint"), 12, "text-faint"));
        Form.Children.Add(new Border { Child = intro, MinHeight = 66, BorderBrush = Design.Brush("border"), BorderThickness = new Thickness(0, 0, 0, 1) });
        if (storageError != null)
            Form.Children.Add(Warn(L("无法刷新存储用量。", "Could not refresh storage usage.") + " " + storageError));
        if (StorageBrowser.Snapshot is not { } usage)
        {
            if (storageError == null) Form.Children.Add(ModelHint(T("settings.storage.loading")));
            return;
        }
        var selection = usage.Select(StorageBrowser.ProjectId);
        var block = new StackPanel { Spacing = 10, Margin = new Thickness(0, 12, 0, 0) }; Form.Children.Add(block);
        if (usage.Projects.Count > 0)
        {
            var projects = new StackPanel { Spacing = 8 }; block.Children.Add(projects);
            var heading = new Grid { ColumnSpacing = 12 };
            heading.ColumnDefinitions.Add(new() { Width = GridLength.Auto }); heading.ColumnDefinitions.Add(new() { Width = new GridLength(1, GridUnitType.Star) });
            var label = Design.Text(T("settings.storage.projects"), 13); label.FontWeight = Microsoft.UI.Text.FontWeights.SemiBold; heading.Children.Add(label);
            var hint = StorageText(T("settings.storage.select_project"), 12, "text-faint"); hint.HorizontalAlignment = HorizontalAlignment.Right; hint.VerticalAlignment = VerticalAlignment.Bottom; Grid.SetColumn(hint, 1); heading.Children.Add(hint); projects.Children.Add(heading);
            var list = storageTiles = new Grid { ColumnSpacing = 8, RowSpacing = 8 }; projects.Children.Add(list);
            list.Children.Add(StorageProjectButton(null, T("settings.storage.all_projects"), "", usage.TotalBytes));
            foreach (var project in usage.Projects) list.Children.Add(StorageProjectButton(project.Id, project.Name, project.Path, project.Bytes));
            void Arrange()
            {
                var columns = NativeStoragePresentation.Columns(list.ActualWidth, list.Children.Count);
                if (columns == list.ColumnDefinitions.Count) return;
                list.ColumnDefinitions.Clear(); list.RowDefinitions.Clear();
                for (var column = 0; column < columns; column++) list.ColumnDefinitions.Add(new() { Width = new GridLength(1, GridUnitType.Star) });
                for (var row = 0; row < (list.Children.Count + columns - 1) / columns; row++) list.RowDefinitions.Add(new() { Height = GridLength.Auto });
                for (var i = 0; i < list.Children.Count; i++) { Grid.SetColumn((FrameworkElement)list.Children[i], i % columns); Grid.SetRow((FrameworkElement)list.Children[i], i / columns); }
            }
            list.Loaded += (_, _) => Arrange(); list.SizeChanged += (_, _) => Arrange();
        }
        var path = StorageText(selection.Path, 12, "text-muted"); path.FontFamily = Design.CodeFont; path.IsTextSelectionEnabled = true;
        block.Children.Add(new Border { Child = path, Padding = new Thickness(10, 8, 10, 8), Background = Design.Brush("bg-sunken"), BorderBrush = Design.Brush("border"), BorderThickness = new Thickness(1), CornerRadius = new CornerRadius(10) });
        if (selection.ProjectId != null) block.Children.Add(ModelHint(T("settings.storage.project_hint")));
        var segments = new Grid();
        foreach (var entry in selection.Entries.Where(entry => entry.Bytes > 0))
        {
            var fraction = NativeStoragePresentation.Fraction(entry.Bytes, selection.TotalBytes);
            if (fraction <= 0) continue;
            segments.ColumnDefinitions.Add(new() { Width = new GridLength(fraction, GridUnitType.Star) });
            var segment = new Border { Background = Design.Brush(entry.ColorToken), MinWidth = 2 };
            ToolTipService.SetToolTip(segment, T(entry.LabelKey) + " " + NativeStoragePresentation.FormatBytes(entry.Bytes));
            Grid.SetColumn(segment, segments.ColumnDefinitions.Count - 1); segments.Children.Add(segment);
        }
        var remaining = Math.Max(0, 1 - selection.Entries.Sum(entry => NativeStoragePresentation.Fraction(entry.Bytes, selection.TotalBytes)));
        if (remaining > 0) segments.ColumnDefinitions.Add(new() { Width = new GridLength(remaining, GridUnitType.Star) });
        block.Children.Add(new Border { Child = segments, Height = 14, Background = Design.Brush("bg-sunken"), BorderBrush = Design.Brush("border"), BorderThickness = new Thickness(1), CornerRadius = new CornerRadius(8) });
        var legend = new StackPanel(); block.Children.Add(legend);
        foreach (var entry in selection.Entries) legend.Children.Add(StorageLegendRow(T(entry.LabelKey), entry.Bytes, entry.ColorToken));
        legend.Children.Add(StorageLegendRow(T("settings.storage.total"), selection.TotalBytes, null));
    }
    private TextBlock StorageText(string text, double size, string color)
    { var block = Design.Text(text, size); block.Foreground = Design.Brush(color); return block; }

    private ToggleButton StorageProjectButton(string? id, string name, string path, ulong bytes)
    {
        var active = StorageBrowser.ProjectId == id;
        var content = new Grid { ColumnSpacing = 12 };
        content.ColumnDefinitions.Add(new() { Width = new GridLength(1, GridUnitType.Star) }); content.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
        var copy = new StackPanel { Spacing = 2, VerticalAlignment = VerticalAlignment.Center };
        var title = Design.Text(name, 13); title.FontWeight = Microsoft.UI.Text.FontWeights.SemiBold; title.TextWrapping = TextWrapping.NoWrap; title.TextTrimming = TextTrimming.CharacterEllipsis; copy.Children.Add(title);
        if (path.Length > 0)
        {
            var location = StorageText(path, 12, "text-faint"); location.FontFamily = Design.CodeFont; location.TextWrapping = TextWrapping.NoWrap; location.TextTrimming = TextTrimming.CharacterEllipsis; ToolTipService.SetToolTip(location, path); copy.Children.Add(location);
        }
        content.Children.Add(copy); var size = StorageText(NativeStoragePresentation.FormatBytes(bytes), 12, "text-muted"); size.VerticalAlignment = VerticalAlignment.Center; Grid.SetColumn(size, 1); content.Children.Add(size);
        var button = new ToggleButton { Content = content, Tag = id ?? "", IsChecked = active, Padding = new Thickness(10, 9, 10, 9), CornerRadius = new CornerRadius(10), BorderThickness = new Thickness(1), HorizontalAlignment = HorizontalAlignment.Stretch, HorizontalContentAlignment = HorizontalAlignment.Stretch, VerticalAlignment = VerticalAlignment.Stretch, MinWidth = 0, MinHeight = 0,
            Background = Design.Brush(active ? "clay-soft" : "bg-sunken"), BorderBrush = Design.Brush(active ? "clay" : "border"), Foreground = Design.Brush("text"), FocusVisualPrimaryBrush = Design.Brush("clay") };
        foreach (var state in new[] { "", "PointerOver", "Pressed", "Checked", "CheckedPointerOver", "CheckedPressed" })
        {
            button.Resources["ToggleButtonBackground" + state] = Design.Brush(active ? "clay-soft" : state is "PointerOver" or "Pressed" ? "bg-elev" : "bg-sunken");
            button.Resources["ToggleButtonBorderBrush" + state] = Design.Brush(active ? "clay" : state is "PointerOver" or "Pressed" ? "border-strong" : "border");
            button.Resources["ToggleButtonForeground" + state] = Design.Brush("text");
        }
        AutomationProperties.SetName(button, name + ", " + NativeStoragePresentation.FormatBytes(bytes)); AutomationProperties.SetHelpText(button, path);
        button.Loaded += (_, _) =>
        {
            if (id == storageFocusProject && storageSelectionFocus != FocusState.Unfocused)
            { var focus = storageSelectionFocus; storageSelectionFocus = FocusState.Unfocused; button.Focus(focus); }
        };
        button.Click += (_, _) => { storageSelectionFocus = button.FocusState; storageFocusProject = id; StorageBrowser.Select(id); Render(); };
        return button;
    }
    private Border StorageLegendRow(string label, ulong bytes, string? color)
    {
        var row = new Grid { ColumnSpacing = 8, MinHeight = 33 };
        row.ColumnDefinitions.Add(new() { Width = new GridLength(10) }); row.ColumnDefinitions.Add(new() { Width = new GridLength(1, GridUnitType.Star) }); row.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
        if (color != null) row.Children.Add(new Border { Width = 10, Height = 10, CornerRadius = new CornerRadius(3), Background = Design.Brush(color), VerticalAlignment = VerticalAlignment.Center });
        var name = Design.Text(label, 13); name.VerticalAlignment = VerticalAlignment.Center; Grid.SetColumn(name, 1); row.Children.Add(name);
        var size = StorageText(NativeStoragePresentation.FormatBytes(bytes), 13, color == null ? "text" : "text-muted"); size.VerticalAlignment = VerticalAlignment.Center; Grid.SetColumn(size, 2); row.Children.Add(size);
        if (color == null) name.FontWeight = size.FontWeight = Microsoft.UI.Text.FontWeights.SemiBold;
        return new Border { Child = row, BorderBrush = Design.Brush("border"), BorderThickness = new Thickness(0, 0, 0, color == null ? 0 : 1) };
    }
}
