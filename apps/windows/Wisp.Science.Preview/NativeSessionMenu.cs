using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Controls.Primitives;
using Wisp.ProjectBrowser;

namespace Wisp.Science.Preview;

/// <summary>WebView context-menu geometry, shared by a row and its overflow button.
/// Child flyouts register separately so Escape removes one visual layer.</summary>
internal sealed class NativeSessionMenu : Flyout
{
    private readonly Dictionary<string, Button> entryButtons = [];
    public void SetEntryVisible(string id, bool visible)
    {
        if (entryButtons.TryGetValue(id, out var button)) button.Visibility = visible ? Visibility.Visible : Visibility.Collapsed;
    }
    public NativeSessionMenu(WispDesign design, NativeSessionMenuEntry[] entries, Action<NativeSessionMenuEntry> pick,
        Action<FlyoutBase> register, Action? dismissRoot = null)
    {
        Placement = FlyoutPlacementMode.BottomEdgeAlignedLeft;
        var presenter = new Style(typeof(FlyoutPresenter));
        presenter.Setters.Add(new Setter(Control.PaddingProperty, new Thickness(6)));
        presenter.Setters.Add(new Setter(Control.BackgroundProperty, design.Brush("bg-elev")));
        presenter.Setters.Add(new Setter(Control.BorderBrushProperty, design.Brush("border-strong")));
        presenter.Setters.Add(new Setter(Control.BorderThicknessProperty, new Thickness(1)));
        presenter.Setters.Add(new Setter(Control.CornerRadiusProperty, new CornerRadius(10)));
        presenter.Setters.Add(new Setter(FrameworkElement.MaxHeightProperty, 600d));
        FlyoutPresenterStyle = presenter;
        var body = new StackPanel { MinWidth = 154, MaxWidth = 266 };
        var children = new List<NativeSessionMenu>();
        foreach (var entry in entries)
        {
            var label = entry.LiteralLabel ?? design.T(entry.LabelKey);
            var text = design.Text(label, 13); text.Foreground = design.Brush(entry.Danger ? "err" : "text");
            text.TextWrapping = TextWrapping.NoWrap; text.TextTrimming = TextTrimming.CharacterEllipsis;
            var content = new Grid { ColumnSpacing = 12 }; content.ColumnDefinitions.Add(new()); content.ColumnDefinitions.Add(new() { Width = GridLength.Auto }); content.Children.Add(text);
            var button = new Button { Content = content, HorizontalAlignment = HorizontalAlignment.Stretch, HorizontalContentAlignment = HorizontalAlignment.Stretch, IsEnabled = entry.Enabled };
            entryButtons.TryAdd(entry.Id, button);
            button.Visibility = entry.Visible ? Visibility.Visible : Visibility.Collapsed;
            design.QuietButton(button); button.Padding = new Thickness(10, 8, 10, 8); button.MinWidth = button.MinHeight = 0; button.CornerRadius = new CornerRadius(6);
            button.Resources["ButtonBackgroundPointerOver"] = design.Brush("bg-input"); button.Opacity = entry.Enabled ? 1 : .4;
            AutomationProperties.SetName(button, label); ToolTipService.SetToolTip(button, label);
            if (entry.Children is { Length: > 0 } nested)
            {
                var chevron = design.Icon("chevron-right", 17, design.ColorText("text-faint")); Grid.SetColumn(chevron, 1); content.Children.Add(chevron);
                var submenu = new NativeSessionMenu(design, nested, pick, register, dismissRoot ?? Hide) { Placement = FlyoutPlacementMode.RightEdgeAlignedTop };
                children.Add(submenu); var open = false;
                submenu.Opened += (_, _) => open = true; submenu.Closed += (_, _) => open = false;
                void Show() { if (!open && button.IsEnabled) submenu.ShowAt(button); }
                button.Click += (_, _) => Show(); button.PointerEntered += (_, _) => Show();
                button.KeyDown += (_, e) => { if (e.Key == Windows.System.VirtualKey.Right) { Show(); e.Handled = true; } };
            }
            else
            {
                button.PointerEntered += (_, _) => { foreach (var child in children) child.Hide(); };
                button.Click += (_, _) => { Hide(); dismissRoot?.Invoke(); pick(entry); };
            }
            body.Children.Add(button);
        }
        // FlyoutPresenter already provides a viewport constrained to the space
        // around its anchor. A second ScrollViewer creates competing scrollbars
        // and hides the bottom actions in short windows at large text sizes.
        Content = body;
        Closed += (_, _) => { foreach (var child in children) child.Hide(); };
        register(this);
    }
}
