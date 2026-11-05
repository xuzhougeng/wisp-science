using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Controls.Primitives;
using Microsoft.UI.Xaml.Input;
using Microsoft.UI.Xaml.Media;
using Wisp.ProjectBrowser;

namespace Wisp.Science.Preview;

/// Owns only panel placement and pointer capture; reading and compaction remain
/// on the conversation page. Reparenting never rebuilds or reloads the details.
internal sealed class NativeContextUsageSurface : IDisposable
{
    private readonly FrameworkElement owner;
    private readonly Button trigger;
    private readonly ContentControl slot;
    private readonly WispDesign design;
    private readonly Action close;
    private readonly Action<bool, double> resized;
    private NativeContextPanelLayout Layout => design.ContextUsageLayout;
    private readonly Border card = new() { CornerRadius = new(18), BorderThickness = new(1), Shadow = new ThemeShadow(), Translation = new(0, 0, 16) };
    private readonly Grid container = new();
    private readonly Grid body = new() { RowSpacing = 10 };
    private readonly ScrollViewer viewport = Scroll();
    private readonly ScrollViewer rowsViewport = Scroll();
    private readonly Button grip;
    private Button? dock;
    private Popup? floating;
    private XamlRoot? root;
    private UIElement? inputRoot;
    private bool captured, fitting;
    public bool IsOpen { get; private set; }

    public NativeContextUsageSurface(FrameworkElement owner, Button trigger, ContentControl slot, WispDesign design, Action close, Action<bool, double> resized)
    {
        this.owner = owner; this.trigger = trigger; this.slot = slot; this.design = design; this.close = close; this.resized = resized;
        body.RowDefinitions.Add(new() { Height = GridLength.Auto }); body.RowDefinitions.Add(new());
        viewport.Content = body; viewport.Margin = new(20, 18, 20, 20); container.Children.Add(viewport); card.Child = container;
        grip = design.ToolButton(design.T("context_usage.resize"), "resize");
        grip.Width = grip.Height = 20; grip.MinWidth = grip.MinHeight = 0; grip.Padding = new(2);
        grip.HorizontalAlignment = HorizontalAlignment.Right; grip.VerticalAlignment = VerticalAlignment.Bottom;
        // Keep the hit target inside its parent; a negative margin can paint
        // outside the content bounds but cannot receive pointer input there.
        grip.Margin = new(2); container.Children.Add(grip);
        AutomationProperties.SetAutomationId(grip, "context-usage-resize");
        // ButtonBase handles PointerPressed before ordinary subscribers.
        grip.AddHandler(UIElement.PointerPressedEvent, new PointerEventHandler((_, e) => Begin(e, resize: true)), true);
        grip.KeyDown += (_, e) =>
        {
            if (!Layout.Floating || root == null || Layout.Geometry is not { } geometry) return;
            var change = e.Key switch { Windows.System.VirtualKey.Left => (-16d, 0d), Windows.System.VirtualKey.Right => (16d, 0d),
                Windows.System.VirtualKey.Up => (0d, -16d), Windows.System.VirtualKey.Down => (0d, 16d), _ => (0d, 0d) };
            if (change == (0d, 0d)) return;
            Layout.Geometry = NativeContextPanelGeometry.Clamp(geometry with { Width = geometry.Width + change.Item1, Height = geometry.Height + change.Item2 }, root.Size.Width, root.Size.Height);
            Fit(); SaveGeometry(); e.Handled = true;
        };
        AutomationProperties.SetAutomationId(card, "context-usage-panel");
        owner.SizeChanged += OwnerSizeChanged;
    }
    private static ScrollViewer Scroll() => new() { HorizontalScrollMode = ScrollMode.Disabled, HorizontalScrollBarVisibility = ScrollBarVisibility.Disabled,
        HorizontalContentAlignment = HorizontalAlignment.Stretch, VerticalScrollBarVisibility = ScrollBarVisibility.Auto };

    public void SetContent(StackPanel top, StackPanel rows, Grid head, Button dockButton)
    {
        body.Children.Clear(); rowsViewport.Content = rows;
        body.Children.Add(top); Grid.SetRow(rowsViewport, 1); body.Children.Add(rowsViewport);
        dock = dockButton; dock.Click += (_, _) => Dock();
        head.Background = new SolidColorBrush(Microsoft.UI.Colors.Transparent);
        head.PointerPressed += (_, e) => { if (!InsideButton(e.OriginalSource as DependencyObject, head)) Begin(e, false); };
        head.DoubleTapped += (_, e) => { if (Layout.Floating && !InsideButton(e.OriginalSource as DependencyObject, head)) { Dock(); e.Handled = true; } };
        AutomationProperties.SetName(card, design.T("context_usage.title"));
        AutomationProperties.SetName(grip, design.T("context_usage.resize"));
        ToolTipService.SetToolTip(grip, design.T("context_usage.resize"));
        design.QuietButton(grip); grip.MinWidth = grip.MinHeight = 0; grip.Padding = new(2);
        grip.Content = design.Icon("resize", 16, design.ColorText("text-faint"));
        card.Background = design.Brush("bg-elev"); card.BorderBrush = design.Brush("border-strong");
        card.RequestedTheme = design.Dark ? ElementTheme.Dark : ElementTheme.Light;
        Fit();
    }
    public void Show()
    {
        if (IsOpen || owner.XamlRoot is not { } xamlRoot || xamlRoot.Content is not UIElement content) return;
        IsOpen = true; root = xamlRoot; inputRoot = content;
        root.Changed += RootChanged;
        inputRoot.AddHandler(UIElement.PointerPressedEvent, new PointerEventHandler(OutsidePressed), true);
        inputRoot.AddHandler(UIElement.PointerMovedEvent, new PointerEventHandler(PointerMoved), true);
        inputRoot.AddHandler(UIElement.PointerReleasedEvent, new PointerEventHandler(PointerReleased), true);
        inputRoot.AddHandler(UIElement.PointerCanceledEvent, new PointerEventHandler(PointerCanceled), true);
        inputRoot.AddHandler(UIElement.PointerCaptureLostEvent, new PointerEventHandler(PointerCanceled), true);
        Fit();
    }
    public void Hide()
    {
        if (!IsOpen) return;
        IsOpen = false; EndGesture(save: false);
        if (root != null) root.Changed -= RootChanged;
        if (inputRoot != null)
        {
            inputRoot.RemoveHandler(UIElement.PointerPressedEvent, new PointerEventHandler(OutsidePressed));
            inputRoot.RemoveHandler(UIElement.PointerMovedEvent, new PointerEventHandler(PointerMoved));
            inputRoot.RemoveHandler(UIElement.PointerReleasedEvent, new PointerEventHandler(PointerReleased));
            inputRoot.RemoveHandler(UIElement.PointerCanceledEvent, new PointerEventHandler(PointerCanceled));
            inputRoot.RemoveHandler(UIElement.PointerCaptureLostEvent, new PointerEventHandler(PointerCanceled));
        }
        Detach(); root = null; inputRoot = null;
    }
    private void Detach()
    {
        slot.Content = null; slot.Visibility = Visibility.Collapsed;
        if (floating != null) { floating.IsOpen = false; floating.Child = null; floating = null; }
    }
    private void Dock()
    {
        EndGesture(save: true); Layout.Dock(); Fit();
    }
    public void Fit()
    {
        if (!IsOpen || root == null || fitting) return;
        fitting = true;
        try
        {
            Layout.Fit(root.Size.Width, root.Size.Height);
            if (dock != null) dock.Visibility = Layout.Floating ? Visibility.Visible : Visibility.Collapsed;
            grip.Visibility = Layout.Floating ? Visibility.Visible : Visibility.Collapsed;
            if (Layout.Floating && Layout.Geometry is { } geometry)
            {
                if (floating == null)
                {
                    slot.Content = null; slot.Visibility = Visibility.Collapsed;
                    floating = new Popup { XamlRoot = root, Child = card, IsLightDismissEnabled = false };
                }
                card.Width = geometry.Width; card.Height = geometry.Height; card.MaxHeight = double.PositiveInfinity;
                body.Height = Math.Max(1, geometry.Height - 40);
                viewport.VerticalScrollBarVisibility = ScrollBarVisibility.Disabled; viewport.VerticalScrollMode = ScrollMode.Disabled;
                rowsViewport.VerticalScrollBarVisibility = ScrollBarVisibility.Auto; rowsViewport.VerticalScrollMode = ScrollMode.Auto;
                floating.HorizontalOffset = geometry.X; floating.VerticalOffset = geometry.Y; floating.IsOpen = true;
            }
            else
            {
                if (floating != null) { floating.IsOpen = false; floating.Child = null; floating = null; }
                card.Width = card.Height = body.Height = double.NaN;
                card.MaxHeight = Math.Max(80, Math.Min(root.Size.Height * .46, owner.ActualHeight * .65));
                viewport.VerticalScrollBarVisibility = ScrollBarVisibility.Auto; viewport.VerticalScrollMode = ScrollMode.Auto;
                rowsViewport.VerticalScrollBarVisibility = ScrollBarVisibility.Disabled; rowsViewport.VerticalScrollMode = ScrollMode.Disabled;
                if (!ReferenceEquals(slot.Content, card)) slot.Content = card;
                slot.Visibility = Visibility.Visible;
            }
            resized(Layout.Floating, Layout.Geometry?.Height ?? 360);
        }
        finally { fitting = false; }
    }
    private void OwnerSizeChanged(object sender, SizeChangedEventArgs e) => Fit();
    private void RootChanged(XamlRoot sender, XamlRootChangedEventArgs args) => Fit();
    private void Begin(PointerRoutedEventArgs e, bool resize)
    {
        if (!IsOpen || root == null || inputRoot == null || !e.GetCurrentPoint(null).Properties.IsLeftButtonPressed) return;
        var point = e.GetCurrentPoint(null).Position;
        var location = card.TransformToVisual(root.Content).TransformPoint(new Windows.Foundation.Point());
        var bounds = new NativeContextPanelGeometry(location.X, location.Y, card.ActualWidth, card.ActualHeight);
        if (resize) Layout.BeginResize(point.X, point.Y, bounds); else Layout.BeginMove(point.X, point.Y, bounds);
        // ButtonBase owns the press initially; transfer capture to the stable
        // window root so resizing continues when the pointer leaves the handle.
        if (resize) grip.ReleasePointerCapture(e.Pointer);
        captured = inputRoot.CapturePointer(e.Pointer);
        if (!captured) Layout.End();
        e.Handled = true;
    }
    private void PointerMoved(object sender, PointerRoutedEventArgs e)
    {
        if (!captured || root == null) return;
        var point = e.GetCurrentPoint(null).Position;
        if (Layout.Move(point.X, point.Y, root.Size.Width, root.Size.Height)) Fit();
        e.Handled = true;
    }
    private void PointerReleased(object sender, PointerRoutedEventArgs e) { if (captured) { EndGesture(true); e.Handled = true; } }
    private void PointerCanceled(object sender, PointerRoutedEventArgs e) { if (captured) EndGesture(true); }
    private void EndGesture(bool save)
    {
        var changed = Layout.End(); var release = captured; captured = false;
        if (release) inputRoot?.ReleasePointerCaptures();
        if (save && changed) SaveGeometry();
    }
    private void SaveGeometry() { if (Layout.Geometry is { Valid: true } geometry) design.SaveContextUsageGeometry?.Invoke(geometry); }
    private void OutsidePressed(object sender, PointerRoutedEventArgs e)
    {
        if (!IsOpen || Layout.Floating || captured || Inside(e.OriginalSource as DependencyObject, card) || Inside(e.OriginalSource as DependencyObject, trigger)) return;
        close(); // Do not handle: the original click must still focus/activate its target.
    }
    private static bool Inside(DependencyObject? node, DependencyObject ancestor)
    {
        while (node != null) { if (node == ancestor) return true; node = VisualTreeHelper.GetParent(node); } return false;
    }
    private static bool InsideButton(DependencyObject? node, DependencyObject stop)
    {
        while (node != null && node != stop) { if (node is ButtonBase) return true; node = VisualTreeHelper.GetParent(node); } return false;
    }
    public void Dispose() { Hide(); owner.SizeChanged -= OwnerSizeChanged; }
}
