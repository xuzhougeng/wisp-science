using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Media;
using Wisp.ProjectBrowser;

namespace Wisp.Science.Preview;

/// <summary>The shared conversation/file strip mounted in the workspace topbar.</summary>
internal sealed class NativeCenterTabs : UserControl
{
    private readonly WispDesign design;
    private readonly StackPanel headers = new() { Orientation = Orientation.Horizontal, Spacing = 4 };
    private readonly Dictionary<string, TabView> views = [];
    private string? signature;
    private Button? activeButton;
    private bool restoreFocus, revealActive;
    private double paneWidth;
    public NativeCenterTabs(WispDesign design)
    {
        this.design = design;
        Content = new ScrollViewer { Content = headers, HorizontalScrollMode = ScrollMode.Enabled,
            HorizontalScrollBarVisibility = ScrollBarVisibility.Hidden, VerticalScrollMode = ScrollMode.Disabled,
            VerticalScrollBarVisibility = ScrollBarVisibility.Disabled, HorizontalContentAlignment = HorizontalAlignment.Left };
        Height = 34; IsTabStop = false;
        AutomationProperties.SetAutomationId(this, "center-tabs");
        SizeChanged += (_, _) => { revealActive = true; Fit(); };
        LayoutUpdated += (_, _) =>
        {
            if (activeButton is not { IsLoaded: true, ActualWidth: > 0 } active) return;
            if (revealActive) { revealActive = false; active.StartBringIntoView(new BringIntoViewOptions { AnimationDesired = false }); }
            if (restoreFocus && active.Focus(FocusState.Programmatic)) restoreFocus = false;
        };
        Unloaded += (_, _) => restoreFocus = false;
    }
    public void SetPaneWidth(double width) { paneWidth = width; Fit(); }
    public void Present(string title, IReadOnlyList<NativeArtifactTab> files, string? selected,
        Action conversation, Action<string> select, Action<string> close)
    {
        var next = System.Text.Json.JsonSerializer.Serialize(new { title, selected, locale = design.Localization.Locale,
            accent = design.ColorText("clay"), border = design.ColorText("border"), files = files.Select(f => new { f.Key, f.Item.Name }) });
        if (signature == next) return;
        signature = next;
        activeButton = null;
        revealActive = true;
        var keys = files.Select(f => f.Key).Prepend("").ToHashSet(StringComparer.Ordinal);
        foreach (var key in views.Keys.Where(key => !keys.Contains(key)).ToArray())
        { headers.Children.Remove(views[key].Row); views.Remove(key); }
        UpdateTab("", title, selected == null, conversation, null, "center-conversation-tab");
        foreach (var tab in files)
            UpdateTab(tab.Key, tab.Item.Name, selected == tab.Key, () => select(tab.Key), () => close(tab.Key), "center-file-tab");
        Fit();
    }
    private sealed class TabView(Grid row, Button button, TextBlock label)
    {
        public Grid Row = row;
        public Button Button = button;
        public TextBlock Label = label;
        public Button? Dismiss;
        public bool Active, Hovering;
        public void UpdateClose()
        {
            if (Dismiss != null) Dismiss.Opacity = Hovering || Button.FocusState != FocusState.Unfocused || Dismiss.FocusState != FocusState.Unfocused ? .72 : Active ? .55 : 0;
        }
    }
    private void UpdateTab(string key, string title, bool active, Action select, Action? close, string id)
    {
        if (!views.TryGetValue(key, out var view))
        { view = Tab(title, select, close, id); views.Add(key, view); headers.Children.Add(view.Row); }
        view.Active = active; view.Label.Text = title;
        view.Label.Foreground = design.Brush(active ? "text" : "text-muted");
        view.Button.Background = active ? design.Brush("bg-elev") : new SolidColorBrush(Microsoft.UI.Colors.Transparent);
        view.Button.BorderBrush = active ? ActiveBorder() : new SolidColorBrush(Microsoft.UI.Colors.Transparent);
        AutomationProperties.SetName(view.Button, title); ToolTipService.SetToolTip(view.Button, title);
        AutomationProperties.SetHelpText(view.Button, active ? design.L("当前标签", "Selected tab") : design.L("切换标签", "Switch tab"));
        if (view.Dismiss != null)
        { var label = design.T("center.close_tab") + " · " + title; AutomationProperties.SetName(view.Dismiss, label); ToolTipService.SetToolTip(view.Dismiss, label); }
        view.UpdateClose();
        if (active) activeButton = view.Button;
    }
    private TabView Tab(string title, Action select, Action? close, string id)
    {
        var row = new Grid { Height = 32, VerticalAlignment = VerticalAlignment.Center };
        var label = design.Text(title, 12); label.TextWrapping = TextWrapping.NoWrap; label.TextTrimming = TextTrimming.CharacterEllipsis;
        label.CharacterSpacing = -5; label.FontWeight = Microsoft.UI.Text.FontWeights.Medium;
        var button = new Button { Content = label, Height = 32, MinHeight = 32, MinWidth = 0,
            Padding = close == null ? new(14, 0, 14, 0) : new(12, 0, 30, 0), CornerRadius = new(8),
            HorizontalAlignment = HorizontalAlignment.Stretch, HorizontalContentAlignment = HorizontalAlignment.Center,
            BorderThickness = new(1) };
        var view = new TabView(row, button, label);
        button.Resources["ButtonBackgroundPointerOver"] = design.Brush("surface-hover");
        button.Resources["ButtonForegroundPointerOver"] = design.Brush("text");
        button.Click += (_, _) => { select(); RestoreTabFocus(); };
        AutomationProperties.SetName(button, title); AutomationProperties.SetAutomationId(button, id);
        ToolTipService.SetToolTip(button, title); row.Children.Add(button);
        if (close != null)
        {
            var dismiss = design.ToolButton(design.T("center.close_tab") + " · " + title, "close");
            dismiss.Content = design.Icon("close", 12); dismiss.Width = dismiss.Height = 20;
            dismiss.MinWidth = dismiss.MinHeight = 0; dismiss.Padding = new(0); dismiss.CornerRadius = new(8);
            dismiss.HorizontalAlignment = HorizontalAlignment.Right; dismiss.VerticalAlignment = VerticalAlignment.Top;
            dismiss.Margin = new(0, 6, 5, 0); view.Dismiss = dismiss;
            dismiss.Click += (_, _) => { close(); RestoreTabFocus(); };
            row.PointerEntered += (_, _) => { view.Hovering = true; view.UpdateClose(); };
            row.PointerExited += (_, _) => { view.Hovering = false; view.UpdateClose(); };
            row.GotFocus += (_, _) => view.UpdateClose(); row.LostFocus += (_, _) => view.UpdateClose();
            row.Children.Add(dismiss);
        }
        return view;
    }
    // New tab buttons are not focusable until the replacement layout completes.
    private void RestoreTabFocus() { restoreFocus = activeButton?.Focus(FocusState.Programmatic) != true; InvalidateArrange(); }
    private SolidColorBrush ActiveBorder()
    {
        var accent = design.Brush("clay").Color; var border = design.Brush("border").Color;
        var front = accent.A * .24; var back = border.A * .76; var alpha = front + back;
        byte Mix(byte first, byte second) => alpha == 0 ? (byte)0 : (byte)Math.Round((first * front + second * back) / alpha);
        return new(Windows.UI.Color.FromArgb((byte)Math.Round(alpha), Mix(accent.R, border.R), Mix(accent.G, border.G), Mix(accent.B, border.B)));
    }
    private void Fit()
    {
        if (headers.Children.Count == 0) return;
        var widths = CenterTabLayout.Widths(ActualWidth, paneWidth, headers.Children.Count - 1);
        for (var i = 0; i < widths.Length; i++) ((FrameworkElement)headers.Children[i]).Width = widths[i];
    }
}
