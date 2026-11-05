using System.Text.Json;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Controls.Primitives;
using Microsoft.UI.Xaml.Input;
using Microsoft.UI.Xaml.Media;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

namespace Wisp.Science.Preview;

internal sealed partial class NativeConversationPage
{
    private Border? runtimeEnvironmentView;
    private Popup? runtimePinnedPopup;
    private Grid? runtimeEnvironmentHeader;
    private StackPanel? runtimeEnvironmentRows;
    private ScrollViewer? runtimeEnvironmentScroll;
    private TextBox? runtimeEnvironmentFilter;
    private Button? runtimeEnvironmentRefresh, runtimeEnvironmentPin, runtimeEnvironmentReturnFocus;
    private ContentControl? runtimeEnvironmentStatus;
    private SettingsTextMenus? runtimeEnvironmentMenus;
    private string? runtimeEnvironmentKey, runtimeEnvironmentRowsKey;
    private XamlRoot? runtimePinnedRoot;
    private Windows.Foundation.TypedEventHandler<XamlRoot, XamlRootChangedEventArgs>? runtimePinnedResize;
    private double runtimePinnedX, runtimePinnedY;
    private (double X, double Y, double OriginX, double OriginY)? runtimeDrag;

    private async Task OpenRuntimeEnvironment(string context, string language, Button trigger)
    {
        runtimeEnvironmentReturnFocus = trigger;
        await model.Runtimes.Inspector.OpenAsync(context, language);
    }
    private void DetachRuntimeEnvironment()
    {
        if (runtimeEnvironmentView?.Parent is Panel panel) panel.Children.Remove(runtimeEnvironmentView);
        if (runtimePinnedPopup != null) { runtimePinnedPopup.IsOpen = false; runtimePinnedPopup.Child = null; runtimePinnedPopup = null; }
        if (runtimePinnedRoot != null && runtimePinnedResize != null) runtimePinnedRoot.Changed -= runtimePinnedResize;
        runtimePinnedRoot = null; runtimePinnedResize = null; runtimeDrag = null;
    }
    private void CloseRuntimeEnvironment(bool restoreFocus = true)
    {
        var selected = model.Runtimes.Inspector.Selected;
        model.Runtimes.Inspector.Close();
        var button = runtimeEnvironmentReturnFocus is { IsLoaded: true } original ? original
            : runtimeDialog != null ? runtimeLanguageButtons.GetValueOrDefault(selected?.Language ?? "")
            : runtimeContextButtons.GetValueOrDefault(selected?.Context.Id ?? "");
        runtimeEnvironmentReturnFocus = null;
        if (restoreFocus && !disposed && button is { IsLoaded: true }) button.Focus(FocusState.Programmatic);
    }
    private void RefreshRuntimeEnvironment()
    {
        var inspector = model.Runtimes.Inspector;
        if (disposed || inspector.Selected == null)
        {
            DetachRuntimeEnvironment(); runtimeEnvironmentMenus?.Dispose(); runtimeEnvironmentMenus = null;
            runtimeEnvironmentView = null; runtimeEnvironmentKey = runtimeEnvironmentRowsKey = null;
            runtimeEnvironmentRows = null; runtimeEnvironmentScroll = null; runtimeEnvironmentFilter = null; ResizeRuntimeDialog(); return;
        }
        var slot = inspector.Selected;
        var key = slot.Context.Id + "/" + slot.Language + design.Dark + design.Localization.Locale + design.Typography;
        if (key != runtimeEnvironmentKey)
        {
            DetachRuntimeEnvironment(); runtimeEnvironmentMenus?.Dispose(); runtimeEnvironmentMenus = new();
            runtimeEnvironmentKey = key; runtimeEnvironmentRowsKey = null;
            runtimeEnvironmentView = BuildRuntimeEnvironment(slot);
        }
        if (inspector.Pinned)
        {
            if (runtimePinnedPopup == null && XamlRoot is { } root)
            {
                if (runtimeEnvironmentView!.Parent is Panel panel) panel.Children.Remove(runtimeEnvironmentView);
                runtimePinnedRoot = root; runtimePinnedResize = (_, _) => ResizePinnedRuntime(); root.Changed += runtimePinnedResize;
                runtimePinnedPopup = new Popup { XamlRoot = root, Child = runtimeEnvironmentView, IsLightDismissEnabled = false };
                ResizePinnedRuntime(); runtimePinnedPopup.IsOpen = true;
            }
        }
        else if (runtimeColumns != null && runtimeEnvironmentView!.Parent == null) runtimeColumns.Children.Add(runtimeEnvironmentView);
        ResizeRuntimeDialog();
        if (runtimeEnvironmentFilter!.Text != inspector.Filter) runtimeEnvironmentFilter.Text = inspector.Filter;
        runtimeEnvironmentRefresh!.IsEnabled = inspector.CanRefresh;
        runtimeEnvironmentRefresh.Opacity = inspector.CanRefresh ? 1 : .4;
        runtimeEnvironmentPin!.Background = inspector.Pinned ? design.Brush("clay-soft") : new SolidColorBrush(Microsoft.UI.Colors.Transparent);
        var pinLabel = design.T(inspector.Pinned ? "runtime.unpin_environment" : "runtime.pin_environment");
        ToolTipService.SetToolTip(runtimeEnvironmentPin, pinLabel); AutomationProperties.SetName(runtimeEnvironmentPin, pinLabel);
        runtimeEnvironmentStatus!.Content = RuntimeStatusBadge(slot.Status);
        var rowsKey = JsonSerializer.Serialize(inspector.Rows) + inspector.Error + inspector.EmptyKey + inspector.Loading + inspector.Filter + inspector.Snapshot?.TotalCount;
        ResizeRuntimeEnvironmentRows();
        if (rowsKey == runtimeEnvironmentRowsKey) return;
        runtimeEnvironmentRowsKey = rowsKey; runtimeEnvironmentRows!.Children.Clear();
        if (inspector.Error is { } error) { runtimeEnvironmentRows.Children.Add(EnvironmentEmpty(error, true)); return; }
        if (inspector.Rows.Length == 0) { runtimeEnvironmentRows.Children.Add(EnvironmentEmpty(design.T(inspector.EmptyKey))); return; }
        foreach (var item in inspector.Rows)
        {
            var row = EnvironmentColumns(); row.Padding = new(14, 7, 14, 7); row.MinHeight = 38;
            var summary = string.IsNullOrEmpty(item.Summary) ? "—" : item.Summary;
            var errorRow = item.TypeName.Equals("unavailable", StringComparison.OrdinalIgnoreCase);
            var values = new[] { item.Name, item.TypeName, summary, item.SizeBytes is { } size ? NativeStoragePresentation.FormatBytes(size) : "—" };
            for (var i = 0; i < values.Length; i++)
            {
                var text = design.Text(values[i], 11); text.FontFamily = design.CodeFont;
                text.Foreground = design.Brush(errorRow && i is 1 or 2 ? "err" : i == 0 ? "text" : i == 2 ? "text-muted" : "text-faint");
                text.TextTrimming = TextTrimming.CharacterEllipsis; text.TextWrapping = i == 2 ? TextWrapping.Wrap : TextWrapping.NoWrap;
                text.MaxLines = i == 2 ? errorRow ? 3 : 2 : 1; if (i == 0) text.FontWeight = Microsoft.UI.Text.FontWeights.SemiBold;
                if (i == 3) text.HorizontalAlignment = HorizontalAlignment.Right;
                ToolTipService.SetToolTip(text, values[i]); Grid.SetColumn(text, i); row.Children.Add(text);
            }
            var button = new Button { Content = row, Padding = new(0), HorizontalAlignment = HorizontalAlignment.Stretch, HorizontalContentAlignment = HorizontalAlignment.Stretch,
                Background = new SolidColorBrush(Microsoft.UI.Colors.Transparent), BorderBrush = design.Brush("border"), BorderThickness = new(0, 0, 0, 1), CornerRadius = new(0), MinWidth = 520 };
            AutomationProperties.SetName(button, string.Join(" · ", values)); ToolTipService.SetToolTip(button, design.T("runtime.quote_object"));
            button.Click += (_, _) => ShowRuntimeObjectMenu(button, slot, item);
            runtimeEnvironmentRows.Children.Add(button);
        }
        if (inspector.Rows.Length < inspector.Snapshot!.TotalCount || inspector.Filter.Trim().Length > 0)
        {
            var count = design.Text(design.Localization.Format("runtime.objects_showing", ("shown", inspector.Rows.Length.ToString()), ("total", inspector.Snapshot.TotalCount.ToString())), 10.5);
            count.Margin = new(14, 8, 14, 8); count.Foreground = design.Brush("text-faint"); runtimeEnvironmentRows.Children.Add(count);
        }
    }
    private Border EnvironmentEmpty(string text, bool error = false)
    {
        var label = design.Text(text, 12); label.Foreground = design.Brush(error ? "err" : "text-faint"); label.TextAlignment = TextAlignment.Center;
        label.VerticalAlignment = VerticalAlignment.Center;
        return new Border { Child = label, MinHeight = 180, Padding = new(24), MaxWidth = 620 };
    }
    private static Grid EnvironmentColumns()
    {
        var row = new Grid { ColumnSpacing = 12 };
        row.ColumnDefinitions.Add(new() { Width = new(1.1, GridUnitType.Star), MinWidth = 100 });
        row.ColumnDefinitions.Add(new() { Width = new(.8, GridUnitType.Star), MinWidth = 90 });
        row.ColumnDefinitions.Add(new() { Width = new(1.4, GridUnitType.Star), MinWidth = 130 });
        row.ColumnDefinitions.Add(new() { Width = new(58), MinWidth = 58 }); return row;
    }
    private void ResizeRuntimeEnvironmentRows()
    {
        // A horizontally scrollable surface measures children at infinity. Give the
        // star columns a finite viewport width so long values wrap instead of
        // pushing the type, summary and size columns out of alignment.
        if (runtimeEnvironmentRows != null && runtimeEnvironmentScroll is { ActualWidth: > 0 } scroll)
            runtimeEnvironmentRows.Width = Math.Max(model.Runtimes.Inspector.Rows.Length > 0 ? 520 : 0, scroll.ActualWidth);
    }
    private Border BuildRuntimeEnvironment(NativeRuntimeSlot slot)
    {
        var inspector = model.Runtimes.Inspector; var layout = new Grid();
        for (var i = 0; i < 3; i++) layout.RowDefinitions.Add(new() { Height = GridLength.Auto }); layout.RowDefinitions.Add(new());
        RegisterRuntimeEscape(layout);
        runtimeEnvironmentHeader = new Grid { Padding = new(14, 9, 10, 9), ColumnSpacing = 8, MinHeight = 58 };
        runtimeEnvironmentHeader.ColumnDefinitions.Add(new()); runtimeEnvironmentHeader.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
        var title = new StackPanel { Spacing = 2, VerticalAlignment = VerticalAlignment.Center };
        var heading = design.Text(design.Localization.Format("runtime.environment_title", ("language", slot.LanguageLabel)), 14); heading.FontWeight = Microsoft.UI.Text.FontWeights.Bold;
        heading.TextWrapping = TextWrapping.NoWrap; heading.TextTrimming = TextTrimming.CharacterEllipsis; title.Children.Add(heading);
        var identity = design.Text((sessionReferenceScope?.Invoke()?.ProjectName ?? "") + " · " + slot.Label, 10.5);
        identity.FontFamily = design.CodeFont; identity.Foreground = design.Brush("text-faint"); identity.TextWrapping = TextWrapping.NoWrap; identity.TextTrimming = TextTrimming.CharacterEllipsis; title.Children.Add(identity);
        title.PointerPressed += (_, e) =>
        {
            var point = e.GetCurrentPoint(null); if (!inspector.Pinned || !point.Properties.IsLeftButtonPressed) return;
            runtimeDrag = (point.Position.X, point.Position.Y, runtimePinnedX, runtimePinnedY); title.CapturePointer(e.Pointer); e.Handled = true;
        };
        title.PointerMoved += (_, e) =>
        {
            if (runtimeDrag is not { } start) return; var point = e.GetCurrentPoint(null).Position;
            runtimePinnedX = start.OriginX + point.X - start.X; runtimePinnedY = start.OriginY + point.Y - start.Y; ResizePinnedRuntime(); e.Handled = true;
        };
        title.PointerReleased += (_, e) => { runtimeDrag = null; title.ReleasePointerCapture(e.Pointer); };
        title.PointerCanceled += (_, _) => runtimeDrag = null; runtimeEnvironmentHeader.Children.Add(title);
        var tools = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 4, VerticalAlignment = VerticalAlignment.Center };
        var tabs = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 4 };
        foreach (var sibling in inspector.Siblings)
        {
            var tab = RuntimeButton(sibling.LanguageLabel, 11.5); tab.MinWidth = 54; tab.Padding = new(9, 4, 9, 4); tab.CornerRadius = new(999); tab.BorderThickness = new(0);
            tab.Background = sibling.Language == slot.Language ? design.Brush("bg-elev") : new SolidColorBrush(Microsoft.UI.Colors.Transparent);
            AutomationProperties.SetName(tab, design.Localization.Format("runtime.switch_language", ("language", sibling.LanguageLabel)));
            tab.Click += async (_, _) => { if (sibling.Language != inspector.Selected?.Language) await inspector.OpenAsync(sibling.Context.Id, sibling.Language); }; tabs.Children.Add(tab);
        }
        tools.Children.Add(new Border { Child = tabs, Padding = new(3), CornerRadius = new(999), Background = design.Brush("bg-sunken"), BorderBrush = design.Brush("border"), BorderThickness = new(1) });
        Button Icon(string label, string icon)
        { var b = design.ToolButton(design.T(label), icon); b.Width = b.Height = 30; b.MinWidth = b.MinHeight = 0; b.Padding = new(0); tools.Children.Add(b); return b; }
        runtimeEnvironmentPin = Icon("runtime.pin_environment", "pin"); runtimeEnvironmentPin.Click += (_, _) => ToggleRuntimeEnvironmentPin();
        runtimeEnvironmentStatus = new ContentControl { VerticalAlignment = VerticalAlignment.Center }; tools.Children.Add(runtimeEnvironmentStatus);
        runtimeEnvironmentRefresh = Icon("runtime.inspect_objects", "refresh"); runtimeEnvironmentRefresh.Click += async (_, _) => await inspector.InspectAsync();
        var close = Icon("runtime.close_environment", "close"); close.Click += (_, _) => CloseRuntimeEnvironment();
        Grid.SetColumn(tools, 1); runtimeEnvironmentHeader.Children.Add(tools); layout.Children.Add(runtimeEnvironmentHeader);
        // Keep the controls usable with large text in a narrow floating panel.
        runtimeEnvironmentHeader.RowDefinitions.Add(new() { Height = GridLength.Auto }); runtimeEnvironmentHeader.RowDefinitions.Add(new() { Height = GridLength.Auto });
        runtimeEnvironmentHeader.SizeChanged += (_, e) =>
        {
            var wrap = e.NewSize.Width < 540 && design.FontSize(12) > 14;
            Grid.SetRow(tools, wrap ? 1 : 0); Grid.SetColumn(tools, wrap ? 0 : 1); Grid.SetColumnSpan(tools, wrap ? 2 : 1);
            Grid.SetColumnSpan(title, wrap ? 2 : 1);
        };
        runtimeEnvironmentFilter = new TextBox { Text = inspector.Filter, PlaceholderText = design.T("runtime.objects_filter_ph"), Padding = new(10, 7, 10, 7), MinHeight = 0, CornerRadius = new(6),
            Background = design.Brush("bg-sunken"), Foreground = design.Brush("text"), BorderBrush = design.Brush("border-strong"), BorderThickness = new(1), IsSpellCheckEnabled = false, IsTextPredictionEnabled = false };
        design.BindTypography(runtimeEnvironmentFilter, size: 12); AutomationProperties.SetName(runtimeEnvironmentFilter, design.T("runtime.objects_filter")); runtimeEnvironmentMenus!.Register(runtimeEnvironmentFilter);
        runtimeEnvironmentFilter.TextChanged += (_, _) => inspector.SetFilter(runtimeEnvironmentFilter.Text);
        var filter = new Border { Child = NativeDialogVisuals.Input(runtimeEnvironmentFilter, design), Padding = new(12, 8, 12, 8), BorderBrush = design.Brush("border"), BorderThickness = new(0, 1, 0, 1) };
        Grid.SetRow(filter, 1); layout.Children.Add(filter);
        var headings = EnvironmentColumns(); headings.Padding = new(14, 7, 14, 7); headings.Background = design.Brush("bg-sunken");
        var names = new[] { "runtime.object_name", "runtime.object_type", "runtime.object_value", "runtime.object_size" };
        for (var i = 0; i < names.Length; i++) { var text = design.Text(design.T(names[i]), 10.5); text.Foreground = design.Brush("text-faint"); if (i == 3) text.HorizontalAlignment = HorizontalAlignment.Right; Grid.SetColumn(text, i); headings.Children.Add(text); }
        Grid.SetRow(headings, 2); layout.Children.Add(headings);
        runtimeEnvironmentRows = new StackPanel();
        runtimeEnvironmentScroll = new ScrollViewer { Content = runtimeEnvironmentRows, HorizontalScrollBarVisibility = ScrollBarVisibility.Auto, HorizontalContentAlignment = HorizontalAlignment.Stretch };
        runtimeEnvironmentScroll.SizeChanged += (_, _) => ResizeRuntimeEnvironmentRows();
        Grid.SetRow(runtimeEnvironmentScroll, 3); layout.Children.Add(runtimeEnvironmentScroll);
        return new Border { Child = layout, BorderBrush = design.Brush("border-strong"), BorderThickness = new(1), CornerRadius = new(8), Background = design.Brush("bg-elev"),
            Shadow = new ThemeShadow(), Translation = new System.Numerics.Vector3(0, 0, 32), RequestedTheme = design.Dark ? ElementTheme.Dark : ElementTheme.Light };
    }
    private void ToggleRuntimeEnvironmentPin()
    {
        var inspector = model.Runtimes.Inspector;
        if (inspector.Pinned) { CloseRuntimeEnvironment(); return; }
        if (XamlRoot is not { } root) return;
        inspector.Pinned = true; runtimePinnedX = root.Size.Width - 636; runtimePinnedY = (root.Size.Height - 560) / 2;
        DetachRuntimeEnvironment(); CloseRuntimeDialog(false); RefreshRuntimeEnvironment();
    }
    private void ResizePinnedRuntime()
    {
        if (runtimePinnedRoot == null || runtimePinnedPopup == null || runtimeEnvironmentView == null) return;
        var size = runtimePinnedRoot.Size; (runtimePinnedX, runtimePinnedY) = NativeRuntimeInspectorModel.ClampPosition(runtimePinnedX, runtimePinnedY, size.Width, size.Height);
        runtimePinnedPopup.HorizontalOffset = runtimePinnedX; runtimePinnedPopup.VerticalOffset = runtimePinnedY;
        runtimeEnvironmentView.Width = Math.Min(620, Math.Max(0, size.Width - 32)); runtimeEnvironmentView.Height = Math.Min(560, Math.Max(0, size.Height - 32));
    }
    private void ShowRuntimeObjectMenu(Button trigger, NativeRuntimeSlot slot, NativeRuntimeObject item)
    {
        var text = NativeRuntimeInspectorModel.Quote(slot.LanguageLabel, item); var menu = new MenuFlyout(); runtimeEnvironmentMenus!.Register(menu);
        var add = new MenuFlyoutItem { Text = design.T("selection.add_to_chat"), IsEnabled = model.CanAttach || model.CanPrepareProjectDraft };
        add.Click += (_, _) => { if (model.AddRuntimeQuote(text)) { CloseRuntimeEnvironment(false); CloseRuntimeDialog(false); composer.Focus(FocusState.Programmatic); } }; menu.Items.Add(add);
        var side = new MenuFlyoutItem { Text = design.T("selection.quote_side_chat") };
        side.Click += (_, _) => { CloseRuntimeEnvironment(false); CloseRuntimeDialog(false); quote(text); }; menu.Items.Add(side);
        menu.ShowAt(trigger);
    }
}
