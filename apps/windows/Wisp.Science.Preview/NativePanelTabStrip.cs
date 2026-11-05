using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Controls.Primitives;
using Microsoft.UI.Xaml.Media;
using Microsoft.UI.Xaml.Input;
using Windows.Foundation;
using Wisp.ProjectBrowser.Contracts;

namespace Wisp.Science.Preview;

/// Shared inspector navigation. Reordering and closing an inactive tab only change layout.
internal sealed class NativePanelTabStrip : UserControl, IDisposable
{
    private readonly NativePanelTabs tabs;
    private readonly WispDesign design;
    private readonly Func<string, string> label;
    private readonly Func<string, Task> select;
    private readonly Action layoutChanged, close;
    private readonly StackPanel items = new() { Orientation = Orientation.Horizontal, Spacing = 4 };
    private readonly ScrollViewer scroller;
    private readonly Flyout menu = new() { Placement = FlyoutPlacementMode.BottomEdgeAlignedRight };
    private readonly Dictionary<string, (Grid Row, Button Dismiss, Button Select, TextBlock Text)> rows = [];
    private string rendered = "", selected = "", dragged = "";
    private bool menuOpen, disposed, reveal;
    private int dragRevision;
    public ContentControl ViewModes { get; } = new() { Visibility = Visibility.Collapsed };

    public NativePanelTabStrip(NativePanelTabs tabs, WispDesign design, Func<string, string> label,
        Func<string, Task> select, Action layoutChanged, Action close)
    {
        this.tabs = tabs; this.design = design; this.label = label;
        this.select = select; this.layoutChanged = layoutChanged; this.close = close;
        var root = new Grid { ColumnSpacing = 4, Padding = new(10, 8, 8, 8), Background = design.Brush("bg-sunken"),
            BorderBrush = design.Brush("border"), BorderThickness = new(0, 0, 0, 1) };
        root.ColumnDefinitions.Add(new() { Width = GridLength.Auto }); root.ColumnDefinitions.Add(new()); root.ColumnDefinitions.Add(new() { Width = GridLength.Auto }); root.ColumnDefinitions.Add(new() { Width = GridLength.Auto }); root.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
        var home = Tool(design.T("right.launcher.home"), "compass", 28);
        AutomationProperties.SetAutomationId(home, "right-pane-home");
        home.Click += async (_, _) => await select(NativePanelTabs.Home);
        root.Children.Add(home);
        scroller = new() { Content = items, HorizontalScrollMode = ScrollMode.Enabled, VerticalScrollMode = ScrollMode.Disabled,
            HorizontalScrollBarVisibility = ScrollBarVisibility.Hidden, VerticalScrollBarVisibility = ScrollBarVisibility.Disabled };
        scroller.LayoutUpdated += RevealSelected;
        Grid.SetColumn(scroller, 1); root.Children.Add(scroller);
        var add = Tool(design.T("right.add_tab"), "plus", 28);
        add.BorderBrush = design.Brush("border-strong"); add.BorderThickness = new(1); add.Background = design.Brush("bg-elev");
        add.Click += (_, _) => { BuildMenu(); menuOpen = true; menu.ShowAt(add); };
        Grid.SetColumn(add, 2); root.Children.Add(add);
        var hide = Tool(design.T("right.close"), "close", 28); hide.Click += (_, _) => close();
        Grid.SetColumn(ViewModes, 3); root.Children.Add(ViewModes);
        Grid.SetColumn(hide, 4); root.Children.Add(hide);
        var style = new Style(typeof(FlyoutPresenter));
        style.Setters.Add(new Setter(Control.PaddingProperty, new Thickness(6)));
        style.Setters.Add(new Setter(Control.BackgroundProperty, design.Brush("bg-elev")));
        style.Setters.Add(new Setter(Control.BorderBrushProperty, design.Brush("border-strong")));
        style.Setters.Add(new Setter(Control.BorderThicknessProperty, new Thickness(1)));
        style.Setters.Add(new Setter(Control.CornerRadiusProperty, new CornerRadius(8)));
        menu.FlyoutPresenterStyle = style;
        menu.Closed += (_, _) => menuOpen = false;
        Unloaded += (_, _) => menu.Hide();
        Content = root; Refresh();
    }

    private Button Tool(string name, string icon, double size)
    {
        var button = design.ToolButton(name, icon);
        button.Content = design.Icon(icon, size == 20 ? 12 : 14);
        button.Width = button.Height = button.MinWidth = button.MinHeight = size;
        button.Padding = new(size == 20 ? 4 : 7); button.CornerRadius = new(6);
        return button;
    }

    public void Refresh()
    {
        if (disposed) return;
        var key = string.Join("|", tabs.Open);
        if (key != rendered)
        {
            rendered = key; rows.Clear(); items.Children.Clear(); reveal = true;
            foreach (var id in tabs.Open) AddTab(id);
        }
        if (selected != tabs.Selected) { selected = tabs.Selected; reveal = true; }
        foreach (var (id, row) in rows)
        {
            var active = id == tabs.Selected;
            var title = label(id);
            if (row.Text.Text != title)
            {
                row.Text.Text = title; if (active) reveal = true;
                AutomationProperties.SetName(row.Select, title); ToolTipService.SetToolTip(row.Select, title);
                var dismiss = design.T("right.close_tab") + ": " + title;
                AutomationProperties.SetName(row.Dismiss, dismiss); ToolTipService.SetToolTip(row.Dismiss, dismiss);
            }
            row.Row.Background = active ? design.Brush("bg-elev") : null;
            row.Row.BorderBrush = active ? design.Brush("border-strong") : new SolidColorBrush(Microsoft.UI.Colors.Transparent);
            row.Dismiss.Opacity = active ? .55 : 0;
        }
    }

    private void AddTab(string id)
    {
        var text = design.Text(label(id), 12); text.TextWrapping = TextWrapping.NoWrap; text.TextTrimming = TextTrimming.CharacterEllipsis;
        text.FontWeight = Microsoft.UI.Text.FontWeights.Medium;
        var button = new Button { Content = text, HorizontalAlignment = HorizontalAlignment.Stretch };
        design.QuietButton(button); button.MinHeight = 28; button.MinWidth = 0; button.Padding = new(10, 6, 26, 6);
        AutomationProperties.SetName(button, label(id)); AutomationProperties.SetAutomationId(button, "panel-tab-" + id);
        ToolTipService.SetToolTip(button, label(id));
        var suppressClick = false;
        button.Click += async (_, _) => { if (suppressClick) return; if (tabs.Selected != id) await select(id); Refresh(); };
        var row = new Grid { MaxWidth = 180, BorderThickness = new(1), CornerRadius = new(6), AllowDrop = true };
        row.Children.Add(button);
        var dismiss = Tool(design.T("right.close_tab") + ": " + label(id), "close", 20);
        dismiss.HorizontalAlignment = HorizontalAlignment.Right; dismiss.VerticalAlignment = VerticalAlignment.Center; dismiss.Margin = new(0, 0, 4, 0);
        dismiss.Click += async (_, _) =>
        {
            var active = tabs.Selected == id; tabs.Remove(id); layoutChanged(); Refresh();
            if (tabs.Open.Count == 0) close();
            else if (active) await select(tabs.Selected);
        };
        row.Children.Add(dismiss);
        row.PointerEntered += (_, _) => dismiss.Opacity = .72;
        row.PointerExited += (_, _) => dismiss.Opacity = tabs.Selected == id ? .55 : 0;
        row.GotFocus += (_, _) => dismiss.Opacity = .72;
        row.LostFocus += (_, _) => dismiss.Opacity = tabs.Selected == id ? .55 : 0;
        // Track the captured pointer explicitly: Button consumes the native CanDrag
        // gesture. This is a local reorder, never an external file/data drag.
        Point? pressed = null; var pressedRevision = 0;
        button.AddHandler(UIElement.PointerPressedEvent, new PointerEventHandler((_, e) =>
        {
            var point = e.GetCurrentPoint(items); pressed = point.Properties.IsLeftButtonPressed ? point.Position : null;
            suppressClick = false;
            pressedRevision = dragRevision;
            if (pressed != null) button.CapturePointer(e.Pointer);
        }), true);
        button.AddHandler(UIElement.PointerMovedEvent, new PointerEventHandler((_, e) =>
        {
            var point = e.GetCurrentPoint(items);
            if (pressedRevision != dragRevision || pressed is not { } origin || !point.Properties.IsLeftButtonPressed) return;
            if (Math.Abs(point.Position.X - origin.X) + Math.Abs(point.Position.Y - origin.Y) < 6) return;
            dragged = id; suppressClick = true; Refresh();
            if (TargetAt(point.Position) is { } target && target != id)
            { rows[target].Row.Background = design.Brush("clay-soft"); rows[target].Row.BorderBrush = design.Brush("clay"); }
        }), true);
        button.AddHandler(UIElement.PointerReleasedEvent, new PointerEventHandler((_, e) =>
        {
            pressed = null;
            if (dragged != id) return;
            var target = TargetAt(e.GetCurrentPoint(items).Position); dragged = ""; e.Handled = true;
            if (target != null && target != id) { tabs.Move(id, target); layoutChanged(); }
            Refresh();
        }), true);
        button.PointerCanceled += (_, _) => { pressed = null; dragged = ""; Refresh(); };
        button.PointerCaptureLost += (_, _) => DispatcherQueue.TryEnqueue(() =>
        { if (pressed != null) { pressed = null; dragged = ""; Refresh(); } });
        rows.Add(id, (row, dismiss, button, text)); items.Children.Add(row);
    }

    private string? TargetAt(Point point)
    {
        if (point.X < scroller.HorizontalOffset || point.X > scroller.HorizontalOffset + scroller.ViewportWidth) return null;
        foreach (var (id, value) in rows)
        {
            var origin = value.Row.TransformToVisual(items).TransformPoint(new Point());
            if (point.X >= origin.X && point.X <= origin.X + value.Row.ActualWidth
                && point.Y >= origin.Y && point.Y <= origin.Y + value.Row.ActualHeight) return id;
        }
        return null;
    }

    private void BuildMenu()
    {
        var list = new StackPanel { MinWidth = 200 };
        foreach (var id in tabs.Available)
        {
            var row = new Grid { ColumnSpacing = 12 };
            row.ColumnDefinitions.Add(new()); row.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
            var title = design.Text(label(id), 12.5); title.Foreground = design.Brush(tabs.Open.Contains(id) ? "text-muted" : "text"); row.Children.Add(title);
            if (tabs.Open.Contains(id)) { var check = design.Icon("check", 14); Grid.SetColumn(check, 1); row.Children.Add(check); }
            var button = new Button { Content = row, HorizontalAlignment = HorizontalAlignment.Stretch, HorizontalContentAlignment = HorizontalAlignment.Stretch };
            design.QuietButton(button); button.MinHeight = 0; button.Padding = new(10, 8, 10, 8); button.CornerRadius = new(4);
            AutomationProperties.SetName(button, label(id) + (tabs.Open.Contains(id) ? design.L("，已打开", ", already open") : ""));
            button.Click += async (_, _) => { menu.Hide(); if (tabs.Selected != id || !tabs.Open.Contains(id)) await select(id); Refresh(); };
            list.Children.Add(button);
        }
        menu.Content = new ScrollViewer { Content = list, MaxHeight = 280, HorizontalScrollBarVisibility = ScrollBarVisibility.Disabled };
    }

    private void RevealSelected(object? sender, object e)
    {
        if (!reveal || disposed || !rows.TryGetValue(tabs.Selected, out var active) || scroller.ViewportWidth <= 0 || active.Row.ActualWidth <= 0) return;
        reveal = false;
        var left = active.Row.TransformToVisual(items).TransformPoint(new Point()).X;
        var right = left + active.Row.ActualWidth;
        var offset = scroller.HorizontalOffset;
        if (left < offset) offset = Math.Max(0, left - 4);
        else if (right > offset + scroller.ViewportWidth) offset = right - scroller.ViewportWidth + 4;
        scroller.ChangeView(Math.Clamp(offset, 0, scroller.ScrollableWidth), null, null, true);
    }

    public bool HandleEscape()
    {
        if (menuOpen) { menu.Hide(); return true; }
        if (dragged.Length > 0) { dragged = ""; dragRevision++; Refresh(); return true; }
        return false;
    }
    public void Dispose() { if (disposed) return; disposed = true; menu.Hide(); scroller.LayoutUpdated -= RevealSelected; }
}
