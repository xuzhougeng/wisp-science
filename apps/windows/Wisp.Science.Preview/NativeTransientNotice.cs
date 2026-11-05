using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Automation.Peers;
using Microsoft.UI.Xaml.Controls;

namespace Wisp.Science.Preview;

// WebView's passive copy-toast: window-centered, pointer-transparent, 1.6 seconds.
// It never takes focus or consumes Escape from an open menu/dialog.
internal sealed class NativeTransientNotice
{
    private readonly DispatcherTimer timer = new() { Interval = TimeSpan.FromMilliseconds(1600) };
    private Grid? owner;
    private Border? card;

    public NativeTransientNotice() => timer.Tick += (_, _) => Hide();

    public void Show(Grid windowRoot, WispDesign design, string message)
    {
        Hide();
        var text = design.Text(message, 12.5);
        text.Foreground = design.Brush("bg-elev"); text.FontWeight = Microsoft.UI.Text.FontWeights.SemiBold;
        text.TextAlignment = TextAlignment.Center; text.LineHeight = design.FontSize(12.5) * 1.35;
        card = new Border { Child = text, Background = design.Brush("text"), CornerRadius = new(20),
            Padding = new(14, 8, 14, 8), Margin = new(16, 0, 16, 28), MaxWidth = 560,
            HorizontalAlignment = HorizontalAlignment.Center, VerticalAlignment = VerticalAlignment.Bottom,
            IsHitTestVisible = false, Shadow = new Microsoft.UI.Xaml.Media.ThemeShadow(), Translation = new(0, 0, 12) };
        // XAML does not clamp oversized radii like CSS border-radius: 999px.
        card.SizeChanged += (sender, args) => ((Border)sender).CornerRadius = new(Math.Min(args.NewSize.Width, args.NewSize.Height) / 2);
        AutomationProperties.SetName(card, message);
        AutomationProperties.SetAutomationId(card, "conversation-transient-notice");
        AutomationProperties.SetLiveSetting(card, AutomationLiveSetting.Polite);
        Grid.SetRowSpan(card, Math.Max(1, windowRoot.RowDefinitions.Count));
        Grid.SetColumnSpan(card, Math.Max(1, windowRoot.ColumnDefinitions.Count));
        Canvas.SetZIndex(card, 200);
        owner = windowRoot; owner.Children.Add(card);
        FrameworkElementAutomationPeer.FromElement(card)?.RaiseAutomationEvent(AutomationEvents.LiveRegionChanged);
        timer.Start();
    }

    public void Hide()
    {
        timer.Stop();
        if (card != null) owner?.Children.Remove(card);
        card = null; owner = null;
    }
}
